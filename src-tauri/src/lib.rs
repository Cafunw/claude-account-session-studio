//! Main Tauri v2 application backend library for Claude Account & Session Studio.
//!
//! Provides one-click account switching for Claude Desktop & Claude Code:
//! 1. Closes Claude Desktop (`Claude.exe`) and waits for file locks to release.
//! 2. Captures the outgoing account's Electron auth (`Cookies`, `Local Storage`, `config.json`) & CLI credentials.
//! 3. Copies missing `Code` chat pointers (`local_*.json`) to the other accounts (additions only).
//! 4. Restores the target account's Electron auth & CLI credentials.
//! 5. Relaunches Claude Desktop (`explorer.exe shell:AppsFolder\Claude_...!Claude`).

pub mod account_admin;
pub mod accounts;
pub mod background;
pub mod chat_text;
pub mod backup;
pub mod desktop_switch;
pub mod dpapi;
pub mod format_guard;
pub mod fs_watch;
pub mod health;
pub mod journal;
pub mod paths;
pub mod quota_alert;
pub mod shortcut;
pub mod spending;
pub mod sync_engine;
pub mod tray;
pub mod updater;
pub mod usage_history;

use accounts::{
    discover_accounts_in_root, fetch_live_oauth_usage, token_refresh_allowed, load_studio_config,
    save_studio_config, AccountProfile, AutomationSettings, StoredStudioConfig,
};
use backup::{
    append_journal, create_verified_backup, execute_restore, list_backups, BackupManifest, RestoreResult,
};
use desktop_switch::{
    capture_active_account_state, clear_main_desktop_session_for_new_login,
    close_claude_desktop_and_wait, data_root_has_live_auth, has_saved_desktop_login,
    is_data_root_locked, launch_claude_desktop, restore_target_account_state,
};
use paths::{
    count_cli_transcripts, discover_real_roots, find_desktop_executable, is_claude_desktop_running,
    read_last_known_account, EnvironmentReport,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use sync_engine::{
    execute_sync, pending_for_target, read_sidebar_groups, scan_all_sessions, PendingChat, SessionRecord,
    SidebarGroup, SyncSelection,
};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    pub studio_dir: PathBuf,
    pub backup_storage_dir: PathBuf,
    pub config: Mutex<StoredStudioConfig>,
    /// Held by switch/capture commands so the account watcher never copies auth files mid-operation.
    pub op_lock: Mutex<()>,
    /// Set when the sync selection changes; the account watcher syncs on its next tick.
    pub sync_requested: AtomicBool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDashboardPayload {
    pub environment: EnvironmentReport,
    pub automation: AutomationSettings,
    pub accounts: Vec<AccountProfile>,
    pub sessions: Vec<SessionRecord>,
    pub backups: Vec<BackupManifest>,
    pub last_sync_at: Option<String>,
    pub sidebar_groups: Vec<SidebarGroup>,
    /// Set while `format_guard` blocks all writes (Turkish detail for the banner).
    pub format_warning: Option<String>,
}

fn emit_progress(app: &AppHandle, step: &str) {
    let _ = app.emit("switch-progress", step.to_string());
}

fn build_dashboard_state(state: &AppState) -> FullDashboardPayload {
    build_dashboard_state_ex(state, false, &mut Vec::new())
}

/// `force` skips the 5-minute staleness rule; `failed` collects labels of accounts with saved
/// CLI credentials whose live fetch failed (they keep Claude Desktop's own record).
fn build_dashboard_state_ex(state: &AppState, force: bool, failed: &mut Vec<String>) -> FullDashboardPayload {
    let mut cfg = state.config.lock().unwrap().clone();

    let user_profile = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default"));
    let real_cli_home = user_profile.join(".claude");
    let real_cli_global = user_profile.join(".claude.json");
    let real_cli_creds = real_cli_home.join(".credentials.json");
    let real_cli_projects = real_cli_home.join("projects");

    let real_roots = discover_real_roots();
    let active_root_info = real_roots.first().cloned();
    let active_data_root_path = active_root_info
        .as_ref()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| state.studio_dir.join("fallback-data"));

    let desktop_exe = find_desktop_executable();
    let desktop_running = is_claude_desktop_running(Some(&active_data_root_path));
    let cli_transcript_count = count_cli_transcripts(&real_cli_projects);

    let mut accounts = discover_accounts_in_root(
        &active_data_root_path,
        Some(real_cli_global.as_path()),
        &state.studio_dir,
        &cfg.accounts,
    );
    account_admin::unhide_signed_in(state, &mut cfg, &accounts);
    account_admin::apply_order(&mut accounts, &cfg.automation.account_order);

    // Keep live usage fresh (every 5 min). The CLI's own account is never refreshed (rotating its
    // refresh token would log the CLI out); it keeps that token fresh itself and we mirror it.
    let mut updated_any_usage = false;
    for acc in &mut accounts {
        let stale = force
            || acc.usage.source != "oauth-api"
            || acc
                .usage
                .fetched_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map_or(true, |t| chrono::Utc::now().signed_duration_since(t).num_minutes() >= 5);
        if acc.has_dpapi_credentials && stale {
            let allow = token_refresh_allowed(&state.studio_dir, &acc.account_uuid, &real_cli_global, &real_cli_creds);
            match fetch_live_oauth_usage(&state.studio_dir, &acc.account_uuid, allow) {
                Ok(live_u) => {
                    acc.usage = live_u;
                    updated_any_usage = true;
                }
                Err(_) => failed.push(crate::accounts::display_name(acc)),
            }
        }
    }

    if updated_any_usage || cfg.accounts.len() != accounts.len() {
        cfg.accounts = accounts.clone();
        if let Ok(mut guard) = state.config.lock() {
            guard.accounts = accounts.clone();
            let _ = save_studio_config(&state.studio_dir, &guard);
        }
    }

    usage_history::record(&state.studio_dir, &accounts, chrono::Utc::now().timestamp_millis());

    // After a sync every account holds its own pointer to the same chat; list each chat once
    // (the active account's copy), otherwise the list and count double with every account.
    let active_uuid = accounts
        .iter()
        .find(|a| a.is_active)
        .map(|a| a.account_uuid.clone())
        .unwrap_or_default();
    let mut sessions = scan_all_sessions(&active_data_root_path, &real_cli_projects, &accounts);
    let (sidebar_groups, membership) = read_sidebar_groups(&active_data_root_path, &sessions);
    for s in &mut sessions {
        s.group_id = membership.get(&s.session_id).cloned();
    }
    sessions.sort_by_key(|s| !s.account_uuid.eq_ignore_ascii_case(&active_uuid));
    let mut seen_chats = std::collections::HashSet::new();
    sessions.retain(|s| seen_chats.insert(s.cli_session_id.clone().unwrap_or_else(|| s.session_id.clone())));
    sessions.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
    let backups = list_backups(&state.backup_storage_dir);
    let env_report = EnvironmentReport {
        active_data_root: active_root_info,
        all_discovered_roots: real_roots,
        cli_config_home: real_cli_home.to_string_lossy().into_owned(),
        cli_global_config: real_cli_global.to_string_lossy().into_owned(),
        cli_credentials_path: real_cli_creds.to_string_lossy().into_owned(),
        cli_projects_dir: real_cli_projects.to_string_lossy().into_owned(),
        cli_transcript_count,
        desktop_exe_path: desktop_exe.map(|p| p.to_string_lossy().into_owned()),
        desktop_running,
        backup_storage_dir: state.backup_storage_dir.to_string_lossy().into_owned(),
    };

    FullDashboardPayload {
        environment: env_report,
        automation: cfg.automation,
        accounts,
        sessions,
        backups,
        last_sync_at: cfg.last_sync_at,
        sidebar_groups,
        format_warning: format_guard::check(&active_data_root_path),
    }
}

/// On-demand, read-only health check (hashes every backup, so never on the polling path).
#[tauri::command(async)]
fn run_health_check(state: State<'_, AppState>) -> Result<health::HealthReport, String> {
    let projects = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default"))
        .join(".claude")
        .join("projects");
    let data_root = discover_real_roots()
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| state.studio_dir.join("fallback-data"));
    let accounts = state.config.lock().unwrap().accounts.clone();
    Ok(health::build_report(&data_root, &projects, &accounts, &state.backup_storage_dir))
}

#[tauri::command(async)]
fn get_dashboard_state(app: AppHandle, state: State<'_, AppState>) -> Result<FullDashboardPayload, String> {
    let dash = build_dashboard_state(&state);
    tray::refresh(&app, &dash.accounts);
    Ok(dash)
}

/// Read-only: the account's quota samples for the last `days` days plus the weekly forecast.
#[tauri::command(async)]
fn get_usage_history(state: State<'_, AppState>, account_uuid: String, days: u32) -> Result<usage_history::UsageHistory, String> {
    let acc = state
        .config
        .lock()
        .unwrap()
        .accounts
        .iter()
        .find(|a| a.account_uuid.eq_ignore_ascii_case(&account_uuid))
        .cloned()
        .ok_or("Hesap bulunamadı")?;
    let data_root = discover_real_roots()
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| state.studio_dir.join("fallback-data"));
    let now = chrono::Local::now();
    Ok(usage_history::history_for(&state.studio_dir, &data_root, &acc, days, now.timestamp_millis(), now.offset()))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageRefreshResult {
    dashboard: FullDashboardPayload,
    /// One line per account without live data, already worded for the toast.
    notes: Vec<String>,
}

fn age_tr(iso: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let mins = chrono::Utc::now().signed_duration_since(t).num_minutes().max(1);
    Some(match mins {
        m if m < 60 => format!("{m} dakika"),
        m if m < 1440 => format!("{} saat", m / 60),
        m => format!("{} gün", m / 1440),
    })
}

/// "Limitleri yenile": every account now (token refresh only where `token_refresh_allowed`).
#[tauri::command(async)]
fn refresh_all_usage(app: AppHandle, state: State<'_, AppState>) -> Result<UsageRefreshResult, String> {
    let mut failed = Vec::new();
    let dashboard = build_dashboard_state_ex(&state, true, &mut failed);
    tray::refresh(&app, &dashboard.accounts);
    let mut notes: Vec<String> = failed
        .iter()
        .map(|n| format!("{n}: anlık veri alınamadı (giriş süresi dolmuş olabilir)"))
        .collect();
    for acc in dashboard.accounts.iter().filter(|a| !a.has_dpapi_credentials) {
        let name = crate::accounts::display_name(acc);
        let age = (acc.usage.source == "desktop-history")
            .then(|| acc.usage.fetched_at.as_deref().and_then(age_tr))
            .flatten();
        notes.push(match age {
            Some(age) => format!("{name}: anlık veri yok, Claude Desktop'un {age} önceki kaydı gösteriliyor"),
            None => format!("{name}: anlık veri yok, Claude Desktop'ta da kaydı yok"),
        });
    }
    Ok(UsageRefreshResult { dashboard, notes })
}

/// 1-Click Automated Switch:
/// 1. Closes Claude Desktop automatically.
/// 2. Backs up outgoing account's Desktop & CLI login state + creates SHA-256 backup.
/// 3. Copies missing `Code` chat pointers (`local_*.json`) to the other accounts (additions only).
/// 4. Restores target account's Desktop & CLI login state.
/// 5. Launches Claude Desktop automatically.
#[tauri::command(async)]
fn one_click_switch_account(
    target_account_uuid: String,
    extra_chats: Option<Vec<String>>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    perform_switch(&app, &state, &target_account_uuid, extra_chats.unwrap_or_default())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchPlan {
    pub ask: bool,
    pub pending: Vec<PendingChat>,
}

/// Chats a switch to `target_account_uuid` would not bring along yet; `ask` = show the choice dialog.
#[tauri::command(async)]
fn get_switch_plan(target_account_uuid: String, state: State<'_, AppState>) -> SwitchPlan {
    switch_plan(&state, &target_account_uuid)
}

/// Read-only and light: no usage fetch or dashboard rebuild.
pub(crate) fn switch_plan(state: &AppState, target_account_uuid: &str) -> SwitchPlan {
    let Some(root) = discover_real_roots().into_iter().next() else {
        return SwitchPlan { ask: false, pending: Vec::new() };
    };
    let data_root = PathBuf::from(&root.path);
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let (stored, selection) = state
        .config
        .lock()
        .map(|c| (c.accounts.clone(), SyncSelection::from_settings(&c.automation)))
        .unwrap_or_default();
    let accounts = discover_accounts_in_root(
        &data_root,
        Some(home.join(".claude.json").as_path()),
        &state.studio_dir,
        &stored,
    );
    let pending = pending_for_target(
        &data_root,
        &home.join(".claude").join("projects"),
        &accounts,
        selection.as_ref(),
        target_account_uuid,
    );
    // `selection` is `None` in "all" mode, so pending is empty there.
    SwitchPlan { ask: !pending.is_empty(), pending }
}

/// Adds chats (cli ids) to the approved list, deduped; no targets entry means every account.
fn approve_chats(a: &mut AutomationSettings, extra: Vec<String>) -> bool {
    let before = a.selected_chats.len();
    for c in extra {
        if !c.is_empty() && !a.selected_chats.contains(&c) {
            a.selected_chats.push(c);
        }
    }
    a.selected_chats.len() != before
}

/// Shared by the "Bu hesaba geç" command and the tray menu (which passes no `extra_chats`).
pub(crate) fn perform_switch(
    app: &AppHandle,
    state: &AppState,
    target_account_uuid: &str,
    extra_chats: Vec<String>,
) -> Result<FullDashboardPayload, String> {
    let _op = state.op_lock.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut cfg = state.config.lock().unwrap();
        if approve_chats(&mut cfg.automation, extra_chats) {
            save_studio_config(&state.studio_dir, &cfg)?;
        }
    }
    let dash = build_dashboard_state(state);
    let Some(active_root) = dash.environment.active_data_root else {
        return Err("Claude Desktop veri klasörü bulunamadı.".to_string());
    };
    let data_root = PathBuf::from(&active_root.path);
    let projects_dir = PathBuf::from(&dash.environment.cli_projects_dir);

    let target_acc = dash
        .accounts
        .iter()
        .find(|a| a.account_uuid.eq_ignore_ascii_case(&target_account_uuid))
        .ok_or_else(|| "Hedef hesap bulunamadı.".to_string())?
        .clone();

    let outgoing_uuid = read_last_known_account(&data_root.join("config.json"))
        .or_else(|| dash.accounts.iter().find(|a| a.is_active).map(|a| a.account_uuid.clone()))
        .unwrap_or_default();

    // Step 1: Close Claude Desktop automatically
    emit_progress(&app, "1/5 — Claude Desktop otomatik kapatılıyor...");
    let _ = close_claude_desktop_and_wait(&data_root, Duration::from_secs(6))?;

    // Step 2: Capture outgoing account's Desktop cookies/storage & CLI credentials
    emit_progress(&app, "2/5 — Mevcut hesap oturumu kaydediliyor...");
    if !outgoing_uuid.is_empty() {
        let _ = capture_active_account_state(&data_root, &state.studio_dir, &outgoing_uuid);
    }

    // Ensure target account's session folder exists before syncing
    let target_org_dir = data_root
        .join("claude-code-sessions")
        .join(&target_acc.account_uuid)
        .join(&target_acc.org_uuid);
    let _ = fs::create_dir_all(&target_org_dir);

    // Step 3: Copy missing chats across accounts (never sidebar groups)
    emit_progress(
        &app,
        "3/5 — Tüm sohbetler ve proje klasörleri yeni hesaba aktarılıyor...",
    );
    // A changed Desktop format skips only this step (already journaled), the switch still happens.
    if let Err(e) = execute_sync(
        &data_root,
        &projects_dir,
        &state.backup_storage_dir,
        &dash.accounts,
        false, // never copy sidebar groups: each one shows up as a new section in the other account
        "live",
        true,
        SyncSelection::from_settings(&dash.automation).as_ref(),
    ) {
        if !format_guard::is_guard_error(&e) {
            return Err(e);
        }
        emit_progress(&app, "3/5 — Dosya yapısı değişmiş görünüyor; sohbet aktarımı atlandı.");
    }

    // Step 4: Restore target account's Desktop & CLI state
    emit_progress(
        &app,
        &format!("4/5 — '{}' hesabı aktif ediliyor...", target_acc.label),
    );
    let _ = restore_target_account_state(&data_root, &state.studio_dir, &target_acc.account_uuid)?;

    // Update studio config active flag
    {
        let mut cfg = state.config.lock().unwrap();
        for acc in &mut cfg.accounts {
            acc.is_active = acc
                .account_uuid
                .eq_ignore_ascii_case(&target_acc.account_uuid);
        }
        cfg.last_sync_at = Some(chrono::Utc::now().to_rfc3339());
        let _ = save_studio_config(&state.studio_dir, &cfg);
    }

    append_journal(
        &state.backup_storage_dir,
        "one-click-switch",
        "live",
        &format!(
            "Otomatik geçiş tamamlandı: '{}' ({}) -> Tüm sohbetler senkronize edildi ve Claude Desktop başlatıldı",
            target_acc.label, target_acc.email
        ),
        "ok",
    );

    // Step 5: Relaunch Claude Desktop automatically
    emit_progress(&app, "5/5 — Claude Desktop yeniden açılıyor...");
    let _ = launch_claude_desktop();

    let dash = build_dashboard_state(state);
    tray::refresh(app, &dash.accounts);
    Ok(dash)
}

/// 1-Click "Add New Account via Claude Desktop":
/// 1. Closes Claude Desktop.
/// 2. Saves the current account's Desktop & CLI login state into its vault slot.
/// 3. Clears active session cookies (preserving `Local State` and all sessions) so Claude Desktop
///    opens to a clean login screen.
/// 4. Launches Claude Desktop so the user can sign in to their 2nd account.
#[tauri::command(async)]
fn one_click_start_new_account_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    let _op = state.op_lock.lock().unwrap_or_else(|e| e.into_inner());
    let dash = build_dashboard_state(&state);
    let Some(active_root) = dash.environment.active_data_root else {
        return Err("Claude Desktop veri klasörü bulunamadı.".to_string());
    };
    let data_root = PathBuf::from(&active_root.path);

    let outgoing_uuid = read_last_known_account(&data_root.join("config.json"))
        .or_else(|| dash.accounts.iter().find(|a| a.is_active).map(|a| a.account_uuid.clone()))
        .unwrap_or_default();

    emit_progress(&app, "1/4 — Claude Desktop kapatılıyor...");
    let _ = close_claude_desktop_and_wait(&data_root, Duration::from_secs(6))?;

    emit_progress(&app, "2/4 — Mevcut hesabınız güvenle kaydediliyor...");
    let _ = create_verified_backup(
        &data_root,
        &state.backup_storage_dir,
        "Yeni hesap ekleme öncesi otomatik yedek",
        "live",
    )?;
    if !outgoing_uuid.is_empty() {
        let _ = capture_active_account_state(&data_root, &state.studio_dir, &outgoing_uuid);
    }

    emit_progress(&app, "3/4 — Yeni hesap girişi hazırlanıyor...");
    clear_main_desktop_session_for_new_login(&data_root)?;

    emit_progress(&app, "4/4 — Claude Desktop giriş ekranı açılıyor...");
    let _ = launch_claude_desktop();

    append_journal(
        &state.backup_storage_dir,
        "prepare-new-account-login",
        "live",
        "Mevcut hesap kaydedildi ve yeni hesap girişi için Claude Desktop açıldı.",
        "ok",
    );

    Ok(build_dashboard_state(&state))
}

/// `mode` "all" copies everything; "ask" newly copies only the picked chats, sidebar groups and folders,
/// each into its target accounts (missing key = all). Never removes chats an earlier sync copied.
#[tauri::command(async)]
fn set_sync_selection(
    mode: String,
    chats: Vec<String>,
    groups: Vec<String>,
    folders: Vec<String>,
    targets: std::collections::HashMap<String, Vec<String>>,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    {
        if mode != "ask" && mode != "all" {
            return Err(format!("Bilinmeyen aktarım modu: {mode}"));
        }
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.transfer_mode = mode;
        cfg.automation.selected_chats = chats;
        cfg.automation.selected_groups = groups;
        cfg.automation.selected_folders = folders;
        cfg.automation.selected_targets = targets;
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    state.sync_requested.store(true, Ordering::Relaxed);
    Ok(build_dashboard_state(&state))
}

/// Quota-alert threshold (percent, clamped 50..=100).
#[tauri::command(async)]
fn update_preferences(quota_alert_percent: f64, state: State<'_, AppState>) -> Result<FullDashboardPayload, String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.switch_threshold_percent = quota_alert_percent.clamp(50.0, 100.0);
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    Ok(build_dashboard_state(&state))
}

/// Turns the "limit reset" notification on or off.
#[tauri::command(async)]
fn set_notify_on_reset(enabled: bool, state: State<'_, AppState>) -> Result<FullDashboardPayload, String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.notify_on_reset = enabled;
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    Ok(build_dashboard_state(&state))
}

/// Gives an account a user-chosen name; an empty name resets it.
#[tauri::command(async)]
fn rename_account(
    account_uuid: String,
    label: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    {
        let mut cfg = state.config.lock().unwrap();
        let acc = cfg
            .accounts
            .iter_mut()
            .find(|a| a.account_uuid.eq_ignore_ascii_case(&account_uuid))
            .ok_or_else(|| "Hesap bulunamadı.".to_string())?;
        let clean = label.trim();
        acc.custom_label = (!clean.is_empty()).then(|| clean.to_string());
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    let dash = build_dashboard_state(&state);
    tray::refresh(&app, &dash.accounts);
    Ok(dash)
}

#[tauri::command(async)]
fn create_manual_backup(
    reason: String,
    state: State<'_, AppState>,
) -> Result<BackupManifest, String> {
    let dash = build_dashboard_state(&state);
    let Some(active_root) = dash.environment.active_data_root else {
        return Err("Veri klasörü bulunamadı.".to_string());
    };
    let clean_reason = if reason.trim().is_empty() {
        "Manuel Yedek".to_string()
    } else {
        reason.trim().to_string()
    };
    create_verified_backup(
        Path::new(&active_root.path),
        &state.backup_storage_dir,
        &clean_reason,
        "live",
    )
}

#[tauri::command(async)]
fn execute_safe_restore(
    backup_id: String,
    state: State<'_, AppState>,
) -> Result<RestoreResult, String> {
    let dash = build_dashboard_state(&state);
    let Some(active_root) = dash.environment.active_data_root else {
        return Err("Veri klasörü bulunamadı.".to_string());
    };
    execute_restore(
        &state.backup_storage_dir,
        &backup_id,
        Path::new(&active_root.path),
        "live",
    )
}

/// Copy chats missing in any account (additions only, sidebar config untouched). Returns how many were added.
fn run_background_sync(state: &AppState, data_root: &Path, take_backup: bool) -> Result<usize, String> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let (stored, selection) = state
        .config
        .lock()
        .map(|c| (c.accounts.clone(), SyncSelection::from_settings(&c.automation)))
        .unwrap_or_default();
    let accounts = discover_accounts_in_root(
        data_root,
        Some(home.join(".claude.json").as_path()),
        &state.studio_dir,
        &stored,
    );
    let res = execute_sync(
        data_root,
        &home.join(".claude").join("projects"),
        &state.backup_storage_dir,
        &accounts,
        false,
        "live",
        take_backup,
        selection.as_ref(),
    )?;
    Ok(res.copied_new_files + res.created_conflict_companions + res.refreshed_metadata)
}

/// Watches Claude Desktop's `lastKnownAccountUuid`. When a new account signs in, the UI is told
/// right away and the login is saved into that account's slot as soon as Desktop releases its
/// cookie locks (they are exclusively locked while Desktop runs, so it cannot be copied earlier).
fn spawn_account_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut root: Option<PathBuf> = None;
        let mut last_seen: Option<String> = None;
        let mut pending: Option<String> = None;
        let mut last_sync: Option<Instant> = None;
        let mut last_backup: Option<Instant> = None;
        let mut fs_watch: Option<fs_watch::SessionWatch> = None;
        let mut fs_watch_failed = false;
        let mut sync_requested = false;
        loop {
            std::thread::sleep(Duration::from_secs(4));
            if root.is_none() {
                root = discover_real_roots().first().map(|r| PathBuf::from(&r.path));
            }
            let Some(data_root) = root.as_ref() else { continue };

            if fs_watch.is_none() {
                match fs_watch::SessionWatch::start(data_root) {
                    Ok(w) => fs_watch = Some(w),
                    // sessions dir may not exist yet; retry every tick, log only once
                    Err(e) if !fs_watch_failed => {
                        fs_watch_failed = true;
                        append_journal(&state.backup_storage_dir, "fs-watch", "live", &e, "error");
                    }
                    Err(_) => {}
                }
            }
            sync_requested |= fs_watch.as_ref().is_some_and(|w| w.take_due());
            sync_requested |= state.sync_requested.swap(false, Ordering::Relaxed);

            let current = read_last_known_account(&data_root.join("config.json"));
            let account_changed = current != last_seen;

            // Background sync: keeps every account's chat list complete, so whichever account the
            // user switches to (here or inside Claude Desktop) already has all chats.
            // Always on: the stored auto_sync_enabled flag is ignored (kept only so old configs load).
            let interval = state
                .config
                .lock()
                .map(|c| c.automation.poll_interval_seconds.max(30))
                .unwrap_or(120);
            // With the watcher active, polling is only a fallback (catches files that got their id via Modify).
            let interval = if fs_watch.is_some() { interval.max(600) } else { interval };
            let due = last_sync.map_or(true, |t| t.elapsed() >= Duration::from_secs(interval));
            if account_changed || due || sync_requested {
                if let Ok(_op) = state.op_lock.try_lock() {
                    last_sync = Some(Instant::now());
                    sync_requested = false;
                    // Pure additions never touch existing files; still snapshot at most every 6h.
                    let take_backup =
                        last_backup.map_or(true, |t| t.elapsed() >= Duration::from_secs(6 * 3600));
                    match run_background_sync(&state, data_root, take_backup) {
                        Ok(added) => {
                            if added > 0 {
                                if take_backup {
                                    last_backup = Some(Instant::now());
                                }
                                let _ = app.emit("sessions-synced", added);
                            }
                            if let Ok(mut c) = state.config.lock() {
                                c.last_sync_at = Some(chrono::Utc::now().to_rfc3339());
                            }
                        }
                        Err(e) if format_guard::is_guard_error(&e) => {} // already journaled once
                        Err(e) => append_journal(&state.backup_storage_dir, "auto-sync", "live", &e, "error"),
                    }
                }
            }

            if account_changed {
                // Only accounts with no saved login: known ones are re-captured by the switch flow,
                // and skipping them avoids racing Desktop's relaunch right after our own switch.
                if let Some(uuid) = current.as_ref().filter(|u| !has_saved_desktop_login(&state.studio_dir, u)) {
                    pending = Some(uuid.clone());
                    let _ = app.emit("account-detected", uuid.clone());
                }
                last_seen = current;
            }

            let Some(uuid) = pending.clone() else { continue };
            if last_seen.as_deref() != Some(uuid.as_str()) {
                pending = None; // user moved on to another account before Desktop was closed
                continue;
            }
            if !data_root_has_live_auth(data_root) || is_data_root_locked(data_root) {
                continue;
            }
            let Ok(_op) = state.op_lock.try_lock() else { continue };
            if capture_active_account_state(data_root, &state.studio_dir, &uuid).is_ok() {
                append_journal(
                    &state.backup_storage_dir,
                    "auto-capture-account",
                    "live",
                    &format!("Yeni giriş yapılan hesap otomatik kaydedildi ({uuid})"),
                    "ok",
                );
                pending = None;
                let _ = app.emit("account-saved", uuid);
            }
        }
    });
}

pub fn run() {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default\\AppData\\Local"));

    let studio_dir = local_app_data.join("ClaudeSessionStudio");
    let backup_storage_dir = local_app_data.join("ClaudeSessionSyncBackups");

    let _ = fs::create_dir_all(&studio_dir);
    let _ = fs::create_dir_all(&backup_storage_dir);

    let cfg = load_studio_config(&studio_dir);

    let app_state = AppState {
        studio_dir,
        backup_storage_dir,
        config: Mutex::new(cfg),
        op_lock: Mutex::new(()),
        sync_requested: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            background::show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(app_state)
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            background::remove_legacy_autostart(app.handle());
            if let Some(w) = app.get_webview_window("main") {
                background::show_window(&w);
            }
            let _ = tray::setup(app.handle());
            shortcut::apply_saved(app.handle());
            spawn_account_watcher(app.handle().clone());
            quota_alert::spawn(app.handle().clone());
            format_guard::spawn_alert(app.handle().clone());
            updater::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard_state,
            get_usage_history,
            spending::get_spending,
            refresh_all_usage,
            run_health_check,
            chat_text::search_chats,
            chat_text::export_chat_markdown,
            chat_text::reveal_in_explorer,
            one_click_switch_account,
            one_click_start_new_account_login,
            rename_account,
            account_admin::remove_account,
            account_admin::move_account,
            set_sync_selection,
            get_switch_plan,
            update_preferences,
            journal::get_journal,
            shortcut::set_switch_shortcut,
            updater::check_for_update,
            updater::install_update,
            updater::set_auto_update,
            set_notify_on_reset,
            create_manual_backup,
            execute_safe_restore,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Claude Account & Session Studio");
}
