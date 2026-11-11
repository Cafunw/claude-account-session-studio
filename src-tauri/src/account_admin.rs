//! Removing an account from the app and ordering the account list.
//!
//! Removal deletes only this app's own copies (`<studio>/slots/<uuid>`, `<studio>/credentials/<uuid>.dpapi`)
//! and hides the account; chats and Claude Desktop's data are never touched.

use crate::accounts::{display_name, save_studio_config, AccountProfile, StoredStudioConfig};
use crate::backup::append_journal;
use crate::i18n::t;
use crate::{build_dashboard_state, tray, AppState, FullDashboardPayload};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

/// Strict `8-4-4-4-12` hex form; anything else (paths, `..`, separators) is rejected.
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Hidden unless it is Claude Desktop's signed-in account (the user signed in on purpose).
pub fn is_hidden(hidden: &[String], uuid: &str, signed_in: Option<&str>) -> bool {
    hidden.iter().any(|h| h.eq_ignore_ascii_case(uuid)) && !signed_in.is_some_and(|s| s.eq_ignore_ascii_case(uuid))
}

/// Accounts in `order` first (in that order), the rest after them in their current order.
pub fn apply_order(accounts: &mut [AccountProfile], order: &[String]) {
    accounts.sort_by_key(|a| {
        order
            .iter()
            .position(|o| o.eq_ignore_ascii_case(&a.account_uuid))
            .unwrap_or(usize::MAX)
    });
}

/// Swaps `uuid` with its neighbour; false at the ends or when it is not listed.
fn move_in(order: &mut [String], uuid: &str, up: bool) -> bool {
    let Some(i) = order.iter().position(|o| o.eq_ignore_ascii_case(uuid)) else { return false };
    let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < order.len()) };
    let Some(j) = j else { return false };
    order.swap(i, j);
    true
}

/// Un-hides every signed-in account; true when `hidden` changed.
fn unhide(hidden: &mut Vec<String>, accounts: &[AccountProfile]) -> bool {
    let before = hidden.len();
    hidden.retain(|h| !accounts.iter().any(|a| a.is_active && a.account_uuid.eq_ignore_ascii_case(h)));
    hidden.len() != before
}

/// Keeps the stored hidden list in step with discovery, which already shows a hidden account
/// once it is signed in again (see `accounts::discover_accounts_in_root`).
pub(crate) fn unhide_signed_in(state: &AppState, cfg: &mut StoredStudioConfig, accounts: &[AccountProfile]) {
    if unhide(&mut cfg.automation.hidden_accounts, accounts) {
        if let Ok(mut guard) = state.config.lock() {
            unhide(&mut guard.automation.hidden_accounts, accounts);
            let _ = save_studio_config(&state.studio_dir, &guard);
        }
    }
}

fn removable<'a>(accounts: &'a [AccountProfile], uuid: &str) -> Result<&'a AccountProfile, String> {
    if !is_uuid(uuid) {
        return Err(t("Geçersiz hesap kimliği.", "Invalid account ID.").to_string());
    }
    let acc = accounts
        .iter()
        .find(|a| a.account_uuid.eq_ignore_ascii_case(uuid))
        .ok_or_else(|| t("Hesap bulunamadı.", "Account not found.").to_string())?;
    if acc.is_active {
        return Err(t(
            "Bu hesapla şu an Claude Desktop'ta giriş yapılmış; kaldırmak için önce başka bir hesaba geç.",
            "This account is signed in to Claude Desktop right now; switch to another account before removing it.",
        )
        .to_string());
    }
    Ok(acc)
}

/// `<base>/<name>` resolved on disk; `None` if it does not exist. Refuses anything that does not resolve
/// to a direct child of `base` with that same name (traversal, junctions, links to another slot).
fn checked_child(base: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    let target = base.join(name);
    if fs::symlink_metadata(&target).is_err() {
        return Ok(None);
    }
    let refuse = || {
        let p = target.display();
        t(format!("Güvenlik: {p} uygulama klasörünün dışında, silinmedi."), format!("Safety: {p} is outside the app folder; not deleted."))
    };
    let base = fs::canonicalize(base).map_err(|_| refuse())?;
    let real = fs::canonicalize(&target).map_err(|_| refuse())?;
    let same_name = real.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name));
    if real.parent() != Some(base.as_path()) || !same_name {
        return Err(refuse());
    }
    Ok(Some(real))
}

/// Deletes the app's saved login slot and CLI credential copy for `uuid` (no backup: they are credentials).
fn delete_app_copies(studio_dir: &Path, uuid: &str) -> Result<(), String> {
    if !is_uuid(uuid) {
        return Err(t("Geçersiz hesap kimliği.", "Invalid account ID.").to_string());
    }
    let slot = checked_child(&studio_dir.join("slots"), uuid)?;
    let cred = checked_child(&studio_dir.join("credentials"), &format!("{uuid}.dpapi"))?;
    if let Some(slot) = slot {
        fs::remove_dir_all(&slot).map_err(|e| t(format!("Giriş kopyası silinemedi: {e}"), format!("Couldn't delete the saved login: {e}")))?;
    }
    if let Some(cred) = cred {
        fs::remove_file(&cred).map_err(|e| t(format!("Kimlik kopyası silinemedi: {e}"), format!("Couldn't delete the saved credentials: {e}")))?;
    }
    Ok(())
}

fn forget(cfg: &mut StoredStudioConfig, uuid: &str) {
    cfg.accounts.retain(|a| !a.account_uuid.eq_ignore_ascii_case(uuid));
    cfg.automation.account_order.retain(|o| !o.eq_ignore_ascii_case(uuid));
    if !cfg.automation.hidden_accounts.iter().any(|h| h.eq_ignore_ascii_case(uuid)) {
        cfg.automation.hidden_accounts.push(uuid.to_string());
    }
}

#[tauri::command(async)]
pub fn remove_account(
    account_uuid: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    // Same lock as switch/capture, so the watcher cannot save a login into the slot being deleted.
    let _op = state.op_lock.lock().unwrap_or_else(|e| e.into_inner());
    let dash = build_dashboard_state(&state);
    let acc = removable(&dash.accounts, &account_uuid)?.clone();
    delete_app_copies(&state.studio_dir, &acc.account_uuid)?;
    {
        let mut cfg = state.config.lock().unwrap();
        forget(&mut cfg, &acc.account_uuid);
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    append_journal(
        &state.backup_storage_dir,
        "remove-account",
        "live",
        &format!(
            "'{}' ({}) uygulamadan kaldırıldı: giriş kopyası silindi, sohbetlere dokunulmadı",
            display_name(&acc),
            acc.account_uuid
        ),
        "ok",
    );
    let dash = build_dashboard_state(&state);
    tray::refresh(&app, &dash.accounts);
    Ok(dash)
}

#[tauri::command(async)]
pub fn move_account(
    account_uuid: String,
    direction: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    let up = match direction.as_str() {
        "up" => true,
        "down" => false,
        _ => return Err(t(format!("Bilinmeyen yön: {direction}"), format!("Unknown direction: {direction}"))),
    };
    let mut order: Vec<String> = build_dashboard_state(&state)
        .accounts
        .into_iter()
        .map(|a| a.account_uuid)
        .collect();
    if move_in(&mut order, &account_uuid, up) {
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.account_order = order;
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    let dash = build_dashboard_state(&state);
    tray::refresh(&app, &dash.accounts);
    Ok(dash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{discover_accounts_in_root, UsageMetrics};
    use crate::sync_engine::preview_sync;
    use serde_json::json;

    const A: &str = "aaaa1111-0000-4000-8000-000000000001";
    const B: &str = "bbbb2222-0000-4000-8000-000000000002";
    const C: &str = "cccc3333-0000-4000-8000-000000000003";

    fn acc(uuid: &str, active: bool) -> AccountProfile {
        AccountProfile {
            id: format!("acc-{uuid}"),
            label: uuid[..4].to_string(),
            custom_label: None,
            email: format!("{}@example.com", &uuid[..4]),
            account_uuid: uuid.to_string(),
            org_uuid: format!("org-{}", &uuid[..4]),
            plan_type: String::new(),
            rate_limit_tier: String::new(),
            has_dpapi_credentials: false,
            is_active: active,
            session_count: 0,
            healthy_session_count: 0,
            last_activity_at: None,
            usage: UsageMetrics::default(),
            added_at: String::new(),
        }
    }

    #[test]
    fn uuid_and_path_checks_reject_traversal() {
        assert!(is_uuid(A));
        for bad in ["", "..", "../../Windows", "aaaa1111-0000-4000-8000-00000000000g", "aaaa1111/0000-4000-8000-000000000001",
            "aaaa1111-0000-4000-8000-0000000000011", "..\\aa1111-0000-4000-8000-000000000001"] {
            assert!(!is_uuid(bad), "{bad}");
        }
        let studio = tempfile::tempdir().unwrap();
        assert!(delete_app_copies(studio.path(), "../credentials").is_err());
        let slots = studio.path().join("slots");
        fs::create_dir_all(slots.join(A)).unwrap();
        assert!(checked_child(&slots, "..").is_err(), "parent of slots");
        assert!(checked_child(&slots, A).unwrap().is_some());
        assert!(checked_child(&slots, B).unwrap().is_none(), "missing is fine");
    }

    #[test]
    fn removal_deletes_only_this_accounts_app_copies() {
        let studio = tempfile::tempdir().unwrap();
        let s = studio.path();
        for u in [A, B] {
            fs::create_dir_all(s.join("slots").join(u).join("desktop")).unwrap();
            fs::write(s.join("slots").join(u).join("desktop").join("Cookies"), "x").unwrap();
            fs::create_dir_all(s.join("credentials")).unwrap();
            fs::write(s.join("credentials").join(format!("{u}.dpapi")), "x").unwrap();
        }
        delete_app_copies(s, B).unwrap();
        assert!(!s.join("slots").join(B).exists() && !s.join("credentials").join(format!("{B}.dpapi")).exists());
        assert!(s.join("slots").join(A).join("desktop").join("Cookies").exists());
        assert!(s.join("credentials").join(format!("{A}.dpapi")).exists());
        delete_app_copies(s, B).unwrap(); // already gone: no error

        let mut cfg = StoredStudioConfig { accounts: vec![acc(A, true), acc(B, false)], ..Default::default() };
        cfg.automation.account_order = vec![B.into(), A.into()];
        forget(&mut cfg, B);
        forget(&mut cfg, B);
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.automation.hidden_accounts, vec![B.to_string()]);
        assert_eq!(cfg.automation.account_order, vec![A.to_string()]);
    }

    #[test]
    fn active_account_cannot_be_removed() {
        let accounts = vec![acc(A, true), acc(B, false)];
        crate::i18n::set_test_lang(crate::i18n::Lang::Tr);
        assert!(removable(&accounts, A).unwrap_err().contains("giriş yapılmış"));
        crate::i18n::set_test_lang(crate::i18n::Lang::En);
        assert!(removable(&accounts, A).unwrap_err().contains("signed in"));
        assert_eq!(removable(&accounts, &B.to_uppercase()).unwrap().account_uuid, B);
        assert!(removable(&accounts, C).is_err());
        assert!(removable(&accounts, "../x").is_err());
    }

    #[test]
    fn hidden_account_skipped_by_discovery_and_sync_until_signed_in_again() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("data");
        let studio = temp.path().join("studio");
        let projects = temp.path().join("projects").join("p");
        fs::create_dir_all(&projects).unwrap();
        for u in [A, B] {
            fs::create_dir_all(data_root.join("claude-code-sessions").join(u).join(format!("org-{}", &u[..4]))).unwrap();
        }
        // one chat that exists only in B
        fs::write(projects.join("cli-1.jsonl"), "{}").unwrap();
        let chat = json!({"sessionId": "local_1", "cliSessionId": "cli-1", "title": "t", "createdAt": 1, "lastActivityAt": 2});
        fs::write(data_root.join("claude-code-sessions").join(B).join("org-bbbb").join("local_1.json"), chat.to_string()).unwrap();
        fs::write(data_root.join("config.json"), json!({"lastKnownAccountUuid": A}).to_string()).unwrap();
        let mut cfg = StoredStudioConfig::default();
        cfg.automation.hidden_accounts = vec![B.into()];
        save_studio_config(&studio, &cfg).unwrap();

        let found = discover_accounts_in_root(&data_root, None, &studio, &[]);
        assert_eq!(found.iter().map(|a| a.account_uuid.as_str()).collect::<Vec<_>>(), vec![A]);
        let preview = preview_sync(&data_root, projects.parent().unwrap(), &found, "sandbox", None);
        assert_eq!(preview.copy_new_count, 0, "hidden account is no sync source");

        // the user signs into B in Claude Desktop again
        fs::write(data_root.join("config.json"), json!({"lastKnownAccountUuid": B}).to_string()).unwrap();
        let found = discover_accounts_in_root(&data_root, None, &studio, &[]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|a| a.account_uuid == B && a.is_active));
        assert_eq!(preview_sync(&data_root, projects.parent().unwrap(), &found, "sandbox", None).copy_new_count, 1);
        let mut hidden = cfg.automation.hidden_accounts.clone();
        assert!(unhide(&mut hidden, &found) && hidden.is_empty());
    }

    #[test]
    fn move_and_order() {
        let mut order = vec![A.to_string(), B.to_string(), C.to_string()];
        assert!(!move_in(&mut order, A, true), "top cannot go up");
        assert!(!move_in(&mut order, C, false), "bottom cannot go down");
        assert!(!move_in(&mut order, "nope", true));
        assert!(move_in(&mut order, C, true));
        assert_eq!(order, vec![A, C, B]);
        assert!(move_in(&mut order, A, false));
        assert_eq!(order, vec![C, A, B]);

        // listed first in that order, unlisted after in their current order
        let mut accounts = vec![acc(A, true), acc(B, false), acc(C, false)];
        apply_order(&mut accounts, &[C.to_uppercase()]);
        assert_eq!(accounts.iter().map(|a| a.account_uuid.as_str()).collect::<Vec<_>>(), vec![C, A, B]);
    }
}
