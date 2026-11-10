//! Multi-account registry, Windows DPAPI credential vault and OAuth usage quota fetcher.
//!
//! - The app never switches accounts by itself (it only notifies when quota is nearly full).
//! - Stored OAuth credentials are encrypted at rest via Windows DPAPI (`dpapi_encrypt`)
//!   in an external vault (`%LOCALAPPDATA%\ClaudeSessionStudio\credentials\`) and
//!   are NEVER mixed into session metadata files.

use crate::dpapi::{atomic_write_verified, dpapi_decrypt, dpapi_encrypt};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const OAUTH_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub utilization: f64,
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsageInfo {
    pub is_enabled: bool,
    pub used_credits: f64,
    pub monthly_limit: f64,
    pub currency: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetrics {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    pub seven_day_sonnet: Option<UsageWindow>,
    pub extra_usage: Option<ExtraUsageInfo>,
    pub fetched_at: Option<String>,
    pub source: String, // "oauth-api" | "desktop-history" | "" (no data yet)
    pub rate_limit_notice: Option<String>,
}

/// A session file keeps its last error forever (and sync copies it to every account),
/// so a "you've hit your limit" error only counts while the quota data still shows a full window.
/// Without quota data it counts for 7 days, the longest a limit can last.
fn limit_notice_still_applies(u: &UsageMetrics, error_at_ms: i64) -> bool {
    let pcts: Vec<f64> = [&u.five_hour, &u.seven_day, &u.seven_day_sonnet]
        .into_iter()
        .flatten()
        .map(|w| w.utilization)
        .collect();
    if pcts.is_empty() {
        return Utc::now().timestamp_millis() - error_at_ms < 7 * 24 * 3_600_000;
    }
    pcts.iter().any(|&p| p >= 100.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub id: String,
    pub label: String,
    /// User-chosen name; `label` is auto-filled, so it can't tell the two apart.
    #[serde(default)]
    pub custom_label: Option<String>,
    pub email: String,
    pub account_uuid: String,
    pub org_uuid: String,
    pub plan_type: String,
    pub rate_limit_tier: String,
    pub has_dpapi_credentials: bool,
    pub is_active: bool,
    pub session_count: usize,
    pub healthy_session_count: usize,
    pub last_activity_at: Option<i64>,
    pub usage: UsageMetrics,
    pub added_at: String,
}

/// Name shown in the tray and notifications; same rule as accountName() in src/App.tsx.
/// Desktop-only logins have no readable e-mail ("account-<id>@claude.ai").
pub fn display_name(acc: &AccountProfile) -> String {
    if let Some(custom) = acc.custom_label.as_deref().filter(|s| !s.is_empty()) {
        return custom.to_string();
    }
    if acc.email.starts_with("account-") && acc.email.ends_with("@claude.ai") {
        format!("Claude hesabı {}", acc.account_uuid.chars().take(8).collect::<String>())
    } else {
        acc.email.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSettings {
    /// Legacy: background sync is always on (additions only); kept so old configs load.
    pub auto_sync_enabled: bool,
    /// Legacy: the app never switches accounts by itself; kept so old configs load.
    pub auto_switch_enabled: bool,
    /// Quota-alert threshold percentage (50..=100) for the 5h/7d windows
    pub switch_threshold_percent: f64,
    /// Notify when a quota window resets after being high
    #[serde(default = "default_true")]
    pub notify_on_reset: bool,
    /// Global shortcut that switches to the next account ("" = off).
    #[serde(default)]
    pub switch_shortcut: String,
    /// Background poll interval in seconds
    pub poll_interval_seconds: u64,
    /// Legacy: sidebar groups are never copied (each would show up as a new section); kept so old configs load.
    pub sync_sidebar_groups: bool,
    /// Legacy, unused; kept so old configs load.
    pub confirm_before_desktop_restart: bool,
    /// "ask": only approved chats (picked below) get new copies; the switch asks about the rest.
    /// "all": every chat goes everywhere. Old configs get "ask" on purpose (asking is the safe default).
    #[serde(default = "default_transfer_mode")]
    pub transfer_mode: String,
    /// Checks GitHub Releases in the background and installs updates; off = never contacts GitHub by itself.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Legacy, replaced by `transfer_mode`; kept so old configs load.
    #[serde(default)]
    pub sync_only_selected: bool,
    /// Approved chats (by cliSessionId); with groups/folders below, what "ask" mode copies.
    #[serde(default)]
    pub selected_chats: Vec<String>,
    #[serde(default)]
    pub selected_groups: Vec<String>,
    /// Picked project folders (normalized by `sync_engine::folder_key`); covers chats outside sidebar groups.
    #[serde(default)]
    pub selected_folders: Vec<String>,
    /// Accounts that should have a picked item (`chat:<cli>`, `group:<id>`, `folder:<path>` -> account uuids).
    /// A missing key means every account.
    #[serde(default)]
    pub selected_targets: std::collections::HashMap<String, Vec<String>>,
    /// Accounts removed from the app (uuids); skipped by discovery and sync until signed in again.
    #[serde(default)]
    pub hidden_accounts: Vec<String>,
    /// User-chosen account order (uuids); unlisted accounts follow.
    #[serde(default)]
    pub account_order: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_transfer_mode() -> String {
    "ask".to_string()
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            auto_sync_enabled: true,
            auto_switch_enabled: false,
            switch_threshold_percent: 90.0,
            notify_on_reset: true,
            switch_shortcut: String::new(),
            poll_interval_seconds: 120,
            sync_sidebar_groups: false,
            confirm_before_desktop_restart: true,
            transfer_mode: default_transfer_mode(),
            auto_update: true,
            sync_only_selected: false,
            selected_chats: Vec::new(),
            selected_groups: Vec::new(),
            selected_folders: Vec::new(),
            selected_targets: Default::default(),
            hidden_accounts: Vec::new(),
            account_order: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredStudioConfig {
    /// Legacy (always "live"); kept so old configs load.
    pub environment_mode: String,
    pub automation: AutomationSettings,
    pub accounts: Vec<AccountProfile>,
    pub last_sync_at: Option<String>,
    pub last_backup_at: Option<String>,
    /// Eski sürümlerden kalan alan; artık kullanılmıyor, eski config dosyaları yüklensin diye duruyor.
    #[serde(default)]
    pub autostart_initialized: bool,
}

impl Default for StoredStudioConfig {
    fn default() -> Self {
        Self {
            environment_mode: "live".to_string(),
            automation: AutomationSettings::default(),
            accounts: Vec::new(),
            last_sync_at: None,
            last_backup_at: None,
            autostart_initialized: false,
        }
    }
}

/// Load or initialize the Studio configuration from `%LOCALAPPDATA%\ClaudeSessionStudio\studio-config.json`.
pub fn load_studio_config(studio_dir: &Path) -> StoredStudioConfig {
    let cfg_path = studio_dir.join("studio-config.json");
    if let Ok(raw) = fs::read_to_string(&cfg_path) {
        if let Ok(mut parsed) = serde_json::from_str::<StoredStudioConfig>(&raw) {
            parsed.environment_mode = "live".to_string();
            // Filter out old synthetic sandbox test accounts if they were persisted earlier
            parsed.accounts.retain(|a| !a.id.starts_with("sandbox-acc-"));
            return parsed;
        }
    }
    StoredStudioConfig::default()
}

pub fn save_studio_config(studio_dir: &Path, cfg: &StoredStudioConfig) -> Result<(), String> {
    fs::create_dir_all(studio_dir).map_err(|e| e.to_string())?;
    let cfg_path = studio_dir.join("studio-config.json");
    let bytes = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
    atomic_write_verified(&cfg_path, &bytes).map(|_| ())
}

/// Store DPAPI-encrypted OAuth credentials for an account slot in `%LOCALAPPDATA%\ClaudeSessionStudio\credentials\<account_uuid>.dpapi`.
pub fn store_dpapi_credentials(
    studio_dir: &Path,
    account_uuid: &str,
    credentials_json: &str,
) -> Result<(), String> {
    let cred_dir = studio_dir.join("credentials");
    fs::create_dir_all(&cred_dir).map_err(|e| e.to_string())?;
    let encrypted = dpapi_encrypt(credentials_json)?;
    let path = cred_dir.join(format!("{account_uuid}.dpapi"));
    atomic_write_verified(&path, encrypted.as_bytes()).map(|_| ())
}

/// Load and decrypt DPAPI-encrypted OAuth credentials for an account slot.
pub fn load_dpapi_credentials(studio_dir: &Path, account_uuid: &str) -> Result<String, String> {
    let path = studio_dir
        .join("credentials")
        .join(format!("{account_uuid}.dpapi"));
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("No stored DPAPI credentials for {account_uuid}: {e}"))?;
    dpapi_decrypt(&raw)
}

pub fn has_dpapi_credentials_file(studio_dir: &Path, account_uuid: &str) -> bool {
    studio_dir
        .join("credentials")
        .join(format!("{account_uuid}.dpapi"))
        .is_file()
}

/// Latest usage Claude Desktop itself recorded for `org_uuid` in `plan-usage-history.json`:
/// `{"version":2,"samples":[{"t":ms,"org":"..","u":{"sd":7-day %,"fh":5-hour %}}]}`.
/// No `fh` means the plan has no 5-hour limit. Zero network and no terminal login needed, but Desktop
/// only samples the signed-in account, so other accounts show their last known value.
fn read_local_plan_usage_history(data_root: &Path, org_uuid: &str) -> Option<UsageMetrics> {
    let raw = fs::read_to_string(data_root.join("plan-usage-history.json")).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let samples = val.get("samples").unwrap_or(&val).as_array()?;
    let entry = samples.iter().rev().find(|e| {
        e.get("org")
            .and_then(|v| v.as_str())
            .is_some_and(|o| o.eq_ignore_ascii_case(org_uuid))
    })?;
    let u = entry.get("u")?;
    let win = |k: &str| {
        u.get(k)
            .and_then(|v| v.as_f64())
            .map(|p| UsageWindow { utilization: p, resets_at: None })
    };
    Some(UsageMetrics {
        five_hour: win("fh"),
        seven_day: win("sd"),
        fetched_at: entry
            .get("t")
            .and_then(|v| v.as_i64())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| d.to_rfc3339()),
        source: "desktop-history".to_string(),
        ..Default::default()
    })
}

fn usage_timestamp_ms(u: &UsageMetrics) -> i64 {
    u.fetched_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map_or(0, |t| t.timestamp_millis())
}

/// Scan `<data_root>/claude-code-sessions` and read-only metadata to discover all accounts
/// present in that data root and merge with stored account metadata.
pub fn discover_accounts_in_root(
    data_root: &Path,
    cli_global_config: Option<&Path>,
    studio_dir: &Path,
    stored_accounts: &[AccountProfile],
) -> Vec<AccountProfile> {
    let last_known_uuid = crate::paths::read_last_known_account(&data_root.join("config.json"));
    // Read from disk so every caller (dashboard, sync, switch plan, quota alert) skips them alike.
    let hidden = load_studio_config(studio_dir).automation.hidden_accounts;
    let is_hidden = |uuid: &str| crate::account_admin::is_hidden(&hidden, uuid, last_known_uuid.as_deref());

    // Read-only lookup from `.claude.json` -> `oauthAccount` and `.claude/.credentials.json` if available
    let mut cli_email_by_uuid: HashMap<String, (String, String, String, String)> = HashMap::new();
    if let Some(cli_cfg) = cli_global_config {
        if let Ok(raw) = fs::read_to_string(cli_cfg) {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(oa) = val.get("oauthAccount") {
                    let uuid = oa
                        .get("accountUuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let email = oa
                        .get("emailAddress")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let org = oa
                        .get("organizationUuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    // Plan name exactly as Anthropic reports it (e.g. "Ultra"), so new plans need no code change.
                    let mut plan_label = oa
                        .get("planDisplayName")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| if s.starts_with("Claude") { s.to_string() } else { format!("Claude {s}") })
                        .unwrap_or_else(|| "Claude Max / Pro".to_string());
                    let from_display_name = plan_label != "Claude Max / Pro";
                    let mut tier_str = oa
                        .get("organizationRateLimitTier")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default_claude_max_20x")
                        .to_string();

                    if let Some(parent) = cli_cfg.parent() {
                        let creds_path = parent.join(".claude").join(".credentials.json");
                        if let Ok(creds_raw) = fs::read_to_string(&creds_path) {
                            // Mirror the CLI's current token (it refreshes it itself) so usage stays fetchable.
                            if !uuid.is_empty()
                                && !is_hidden(&uuid)
                                && creds_raw.contains("accessToken")
                                && load_dpapi_credentials(studio_dir, &uuid).ok().as_deref() != Some(creds_raw.as_str())
                            {
                                let _ = store_dpapi_credentials(studio_dir, &uuid, &creds_raw);
                            }
                            if let Ok(cval) = serde_json::from_str::<Value>(&creds_raw) {
                                if let Some(co) = cval.get("claudeAiOauth") {
                                    let sub = co.get("subscriptionType").and_then(|v| v.as_str()).unwrap_or("");
                                    let rlt = co.get("rateLimitTier").and_then(|v| v.as_str()).unwrap_or("");
                                    if !rlt.is_empty() {
                                        tier_str = rlt.to_string();
                                    }
                                    if from_display_name {
                                        // keep the official plan name
                                    } else if rlt.contains("20x") {
                                        plan_label = "Claude Max 20x".to_string();
                                    } else if rlt.contains("5x") {
                                        plan_label = "Claude Max 5x".to_string();
                                    } else if sub.eq_ignore_ascii_case("max") {
                                        plan_label = "Claude Max".to_string();
                                    } else if sub.eq_ignore_ascii_case("pro") {
                                        plan_label = "Claude Pro".to_string();
                                    } else if sub.eq_ignore_ascii_case("team") {
                                        plan_label = "Claude Team".to_string();
                                    }
                                }
                            }
                        }
                    }

                    if !uuid.is_empty() {
                        cli_email_by_uuid.insert(uuid, (email, org, plan_label, tier_str));
                    }
                }
            }
        }
    }

    let stored_by_uuid: HashMap<String, AccountProfile> = stored_accounts
        .iter()
        .map(|a| (a.account_uuid.clone(), a.clone()))
        .collect();

    let mut discovered: Vec<AccountProfile> = Vec::new();
    let sessions_root = data_root.join("claude-code-sessions");

    if let Ok(acc_dirs) = fs::read_dir(&sessions_root) {
        for acc_entry in acc_dirs.flatten() {
            if !acc_entry.path().is_dir() {
                continue;
            }
            let account_uuid = acc_entry.file_name().to_string_lossy().into_owned();
            if is_hidden(&account_uuid) {
                continue;
            }
            let Ok(org_dirs) = fs::read_dir(acc_entry.path()) else {
                continue;
            };

            for org_entry in org_dirs.flatten() {
                if !org_entry.path().is_dir() {
                    continue;
                }
                let org_uuid = org_entry.file_name().to_string_lossy().into_owned();

                let mut session_count = 0usize;
                let mut healthy_count = 0usize;
                let mut newest_activity: Option<i64> = None;
                let mut latest_rate_limit_notice: Option<(i64, String)> = None;

                if let Ok(files) = fs::read_dir(org_entry.path()) {
                    for f in files.flatten() {
                        let fname = f.file_name().to_string_lossy().into_owned();
                        if !fname.starts_with("local_") || !fname.ends_with(".json") {
                            continue;
                        }
                        session_count += 1;
                        if let Ok(raw) = fs::read_to_string(f.path()) {
                            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                                let unavail = v
                                    .get("transcriptUnavailable")
                                    .and_then(|b| b.as_bool())
                                    .unwrap_or(false);
                                if !unavail {
                                    healthy_count += 1;
                                }
                                if let Some(act) = v.get("lastActivityAt").and_then(|n| n.as_i64()) {
                                    if newest_activity.map_or(true, |cur| act > cur) {
                                        newest_activity = Some(act);
                                    }
                                }
                                if let Some(err_str) = v.get("error").and_then(|s| s.as_str()) {
                                    if err_str.to_ascii_lowercase().contains("limit") {
                                        let at = v.get("lastActivityAt").and_then(|n| n.as_i64()).unwrap_or(0);
                                        if latest_rate_limit_notice.as_ref().map_or(true, |(t, _)| at > *t) {
                                            latest_rate_limit_notice = Some((at, err_str.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let existing = stored_by_uuid.get(&account_uuid);
                let cli_meta = cli_email_by_uuid.get(&account_uuid);

                let email = existing
                    .map(|e| e.email.clone())
                    .filter(|s| !s.is_empty())
                    .or_else(|| cli_meta.map(|m| m.0.clone()))
                    .unwrap_or_else(|| format!("account-{}@claude.ai", &account_uuid[..8.min(account_uuid.len())]));

                let label = existing
                    .map(|e| e.label.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| email.split('@').next().unwrap_or("Claude Account").to_string());

                // Fresh CLI login data wins over the stored copy, so a plan change shows up immediately.
                let plan_type = cli_meta
                    .map(|m| m.2.clone())
                    .filter(|s| s != "Claude Max / Pro")
                    .or_else(|| existing.map(|e| e.plan_type.clone()))
                    .filter(|s| !s.is_empty() && s != "Claude Max / Pro")
                    .unwrap_or_else(|| "Plan bilinmiyor".to_string());

                let rate_limit_tier = cli_meta
                    .map(|m| m.3.clone())
                    .or_else(|| existing.map(|e| e.rate_limit_tier.clone()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();

                let is_active = last_known_uuid
                    .as_deref()
                    .map(|u| u.eq_ignore_ascii_case(&account_uuid))
                    .unwrap_or(false);

                let mut usage = existing.map(|e| e.usage.clone()).unwrap_or_default();
                // Desktop's own record covers accounts with no terminal login; live API data
                // (which has reset times) wins while it is not clearly older.
                if let Some(local) = read_local_plan_usage_history(data_root, &org_uuid) {
                    if usage.source != "oauth-api"
                        || usage_timestamp_ms(&local) > usage_timestamp_ms(&usage) + 10 * 60_000
                    {
                        usage = local;
                    }
                }
                // Recomputed every time so a stored notice can't outlive the limit.
                usage.rate_limit_notice = latest_rate_limit_notice
                    .filter(|(at, _)| limit_notice_still_applies(&usage, *at))
                    .map(|(_, e)| e);

                discovered.push(AccountProfile {
                    id: existing
                        .map(|e| e.id.clone())
                        .unwrap_or_else(|| format!("acc-{}", &account_uuid[..8.min(account_uuid.len())])),
                    label,
                    custom_label: existing.and_then(|e| e.custom_label.clone()),
                    email,
                    account_uuid: account_uuid.clone(),
                    org_uuid,
                    plan_type,
                    rate_limit_tier,
                    has_dpapi_credentials: has_dpapi_credentials_file(studio_dir, &account_uuid),
                    is_active,
                    session_count,
                    healthy_session_count: healthy_count,
                    last_activity_at: newest_activity,
                    usage,
                    added_at: existing
                        .map(|e| e.added_at.clone())
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                });
            }
        }
    }

    // Also include any manually registered accounts that don't have a session folder yet
    for stored in stored_accounts {
        if !is_hidden(&stored.account_uuid)
            && !discovered
                .iter()
                .any(|d| d.account_uuid.eq_ignore_ascii_case(&stored.account_uuid))
        {
            let mut copy = stored.clone();
            copy.is_active = last_known_uuid
                .as_deref()
                .map(|u| u.eq_ignore_ascii_case(&copy.account_uuid))
                .unwrap_or(false);
            copy.has_dpapi_credentials = has_dpapi_credentials_file(studio_dir, &copy.account_uuid);
            discovered.push(copy);
        }
    }

    // If none marked active and at least one exists, mark first active
    if !discovered.is_empty() && !discovered.iter().any(|a| a.is_active) {
        discovered[0].is_active = true;
    }

    discovered.sort_by(|a, b| b.is_active.cmp(&a.is_active).then_with(|| a.label.cmp(&b.label)));
    discovered
}

/// Refreshing rotates the refresh token and revokes the old one, so an account the CLI is signed in
/// with must never be refreshed here. `cli_global` / `cli_creds` are `None` when the file is absent;
/// anything unparsable counts as "can't tell" and blocks the refresh.
pub fn may_refresh_token(
    account_uuid: &str,
    stored_creds: &str,
    cli_global: Option<&str>,
    cli_creds: Option<&str>,
) -> bool {
    let refresh_token = |s: &str| -> Option<String> {
        let v: Value = serde_json::from_str(s).ok()?;
        let o = v.get("claudeAiOauth").unwrap_or(&v);
        Some(o.get("refreshToken").and_then(|t| t.as_str()).unwrap_or("").to_string())
    };
    let Some(stored_rt) = refresh_token(stored_creds).filter(|t| !t.is_empty()) else {
        return false;
    };
    let cli_uuid = match cli_global.map(serde_json::from_str::<Value>) {
        None => String::new(),
        Some(Ok(v)) => v
            .pointer("/oauthAccount/accountUuid")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string(),
        Some(Err(_)) => return false,
    };
    if cli_uuid.eq_ignore_ascii_case(account_uuid) {
        return false;
    }
    match cli_creds.map(refresh_token) {
        None => true,
        Some(Some(cli_rt)) => cli_rt != stored_rt,
        Some(None) => false,
    }
}

/// Reads the CLI's files and the stored copy for `may_refresh_token`; any read error means no.
pub fn token_refresh_allowed(studio_dir: &Path, account_uuid: &str, cli_global: &Path, cli_creds: &Path) -> bool {
    let read = |p: &Path| match fs::read_to_string(p) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    };
    let (Ok(stored), Ok(global), Ok(creds)) =
        (load_dpapi_credentials(studio_dir, account_uuid), read(cli_global), read(cli_creds))
    else {
        return false;
    };
    may_refresh_token(account_uuid, &stored, global.as_deref(), creds.as_deref())
}

/// Fetch live usage quota from Anthropic OAuth Usage API (`https://api.anthropic.com/api/oauth/usage`)
/// using an access token decrypted from Windows DPAPI, refreshing the token first if expired.
pub fn fetch_live_oauth_usage(
    studio_dir: &Path,
    account_uuid: &str,
    allow_token_refresh: bool,
) -> Result<UsageMetrics, String> {
    let creds_str = load_dpapi_credentials(studio_dir, account_uuid)?;
    let mut creds_val: Value =
        serde_json::from_str(&creds_str).map_err(|e| format!("Invalid credentials JSON: {e}"))?;

    let oauth = creds_val
        .get("claudeAiOauth")
        .cloned()
        .unwrap_or_else(|| creds_val.clone());

    let mut access_token = oauth
        .get("accessToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let refresh_token = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let expires_at = oauth.get("expiresAt").and_then(|v| v.as_i64()).unwrap_or(0);

    if access_token.is_empty() {
        return Err("Stored credentials do not contain an OAuth accessToken".to_string());
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(12))
        .build();

    let now_ms = Utc::now().timestamp_millis();
    if expires_at > 0 && now_ms >= expires_at && !allow_token_refresh {
        return Err("Stored access token expired".to_string());
    }
    if allow_token_refresh && expires_at > 0 && now_ms + 300_000 >= expires_at && !refresh_token.is_empty() {
        // Refresh OAuth token
        let refresh_resp = agent
            .post(OAUTH_TOKEN_URL)
            .send_json(json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": OAUTH_CLIENT_ID,
            }));

        if let Ok(r) = refresh_resp {
            if let Ok(tok_json) = r.into_json::<Value>() {
                if let Some(new_at) = tok_json.get("access_token").and_then(|v| v.as_str()) {
                    access_token = new_at.to_string();
                    if let Some(obj) = creds_val
                        .get_mut("claudeAiOauth")
                        .and_then(|v| v.as_object_mut())
                    {
                        obj.insert("accessToken".to_string(), json!(access_token));
                        if let Some(new_rt) = tok_json.get("refresh_token").and_then(|v| v.as_str()) {
                            obj.insert("refreshToken".to_string(), json!(new_rt));
                        }
                        if let Some(exp_in) = tok_json.get("expires_in").and_then(|v| v.as_i64()) {
                            obj.insert("expiresAt".to_string(), json!(now_ms + exp_in * 1000));
                        }
                    }
                    if let Ok(updated_str) = serde_json::to_string(&creds_val) {
                        let _ = store_dpapi_credentials(studio_dir, account_uuid, &updated_str);
                    }
                }
            }
        }
    }

    let resp = agent
        .get(OAUTH_USAGE_URL)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", OAUTH_BETA_HEADER)
        .set("User-Agent", "claude-code/2.1.0")
        .call()
        .map_err(|e| format!("OAuth usage request failed: {e}"))?;

    let body: Value = resp
        .into_json()
        .map_err(|e| format!("Invalid usage API JSON: {e}"))?;

    let parse_win = |key: &str| -> Option<UsageWindow> {
        let w = body.get(key)?;
        if w.is_null() {
            return None;
        }
        Some(UsageWindow {
            utilization: w.get("utilization").and_then(|v| v.as_f64()).unwrap_or(0.0),
            resets_at: w
                .get("resets_at")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    };

    let extra_usage = body.get("extra_usage").and_then(|eu| {
        if eu.is_null() {
            None
        } else {
            Some(ExtraUsageInfo {
                is_enabled: eu.get("is_enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                used_credits: eu.get("used_credits").and_then(|v| v.as_f64()).unwrap_or(0.0) / 100.0,
                monthly_limit: eu.get("monthly_limit").and_then(|v| v.as_f64()).unwrap_or(0.0) / 100.0,
                currency: eu
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("USD")
                    .to_string(),
            })
        }
    });

    Ok(UsageMetrics {
        five_hour: parse_win("five_hour"),
        seven_day: parse_win("seven_day"),
        seven_day_sonnet: parse_win("seven_day_sonnet"),
        extra_usage,
        fetched_at: Some(Utc::now().to_rfc3339()),
        source: "oauth-api".to_string(),
        rate_limit_notice: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_prefers_custom_then_email_then_short_uuid() {
        let mut acc: AccountProfile = serde_json::from_str(
            r#"{"id":"","label":"","email":"account-x@claude.ai","accountUuid":"abcdef1234567890","orgUuid":"","planType":"","rateLimitTier":"","hasDpapiCredentials":false,"isActive":false,"sessionCount":0,"healthySessionCount":0,"lastActivityAt":null,"usage":{"source":""},"addedAt":""}"#,
        )
        .unwrap();
        assert_eq!(display_name(&acc), "Claude hesabı abcdef12");
        acc.email = "a@b.com".into();
        assert_eq!(display_name(&acc), "a@b.com");
        acc.custom_label = Some("İş".into());
        assert_eq!(display_name(&acc), "İş");
    }

    #[test]
    fn limit_notice_hidden_once_quota_is_not_full() {
        let w = |p| Some(UsageWindow { utilization: p, resets_at: None });
        let now = Utc::now().timestamp_millis();
        let full = UsageMetrics { seven_day: w(100.0), ..Default::default() };
        let reset = UsageMetrics { seven_day: w(0.0), ..Default::default() };
        assert!(limit_notice_still_applies(&full, now));
        assert!(!limit_notice_still_applies(&reset, now));
        // no quota data: only a recent error counts
        let none = UsageMetrics::default();
        assert!(limit_notice_still_applies(&none, now));
        assert!(!limit_notice_still_applies(&none, now - 8 * 24 * 3_600_000));
    }

    #[test]
    fn desktop_usage_history_takes_latest_sample_of_org() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("plan-usage-history.json"),
            r#"{"version":2,"samples":[
                {"t":1000,"org":"org-a","u":{"sd":40,"fh":12}},
                {"t":2000,"org":"org-b","u":{"sd":5,"fh":1}},
                {"t":3000,"org":"org-a","u":{"sd":55}}]}"#,
        )
        .unwrap();
        let a = read_local_plan_usage_history(dir.path(), "ORG-A").unwrap();
        assert_eq!(a.seven_day.as_ref().unwrap().utilization, 55.0);
        assert!(a.five_hour.is_none(), "no fh = plan without a 5-hour limit");
        assert_eq!(usage_timestamp_ms(&a), 3000);
        assert!(read_local_plan_usage_history(dir.path(), "org-x").is_none());
    }

    #[test]
    fn token_refresh_only_for_accounts_the_cli_is_not_using() {
        let stored = r#"{"claudeAiOauth":{"accessToken":"at","refreshToken":"rt-old"}}"#;
        let global_b = r#"{"oauthAccount":{"accountUuid":"uuid-b"}}"#;
        let creds_b = r#"{"claudeAiOauth":{"accessToken":"x","refreshToken":"rt-b"}}"#;
        assert!(may_refresh_token("uuid-a", stored, Some(global_b), Some(creds_b)));
        assert!(may_refresh_token("uuid-a", stored, None, None), "no CLI login at all");
        // CLI's current account (any case), even with a different token.
        assert!(!may_refresh_token("UUID-B", stored, Some(global_b), Some(creds_b)));
        // Same refresh token as the CLI, even if .claude.json names another account.
        let creds_same = r#"{"claudeAiOauth":{"refreshToken":"rt-old"}}"#;
        assert!(!may_refresh_token("uuid-a", stored, Some(global_b), Some(creds_same)));
        // Half-written CLI files or an unusable stored copy: don't risk it.
        assert!(!may_refresh_token("uuid-a", stored, Some("{"), Some(creds_b)));
        assert!(!may_refresh_token("uuid-a", stored, Some(global_b), Some("not json")));
        assert!(!may_refresh_token("uuid-a", r#"{"claudeAiOauth":{"accessToken":"at"}}"#, None, None));
        assert!(!may_refresh_token("uuid-a", "garbage", None, None));
    }

    #[test]
    fn old_config_without_transfer_mode_loads_as_ask() {
        let mut v = serde_json::to_value(AutomationSettings { transfer_mode: "all".into(), ..Default::default() }).unwrap();
        v.as_object_mut().unwrap().remove("transferMode").unwrap();
        let a: AutomationSettings = serde_json::from_value(v).unwrap();
        assert_eq!(a.transfer_mode, "ask");
    }
}
