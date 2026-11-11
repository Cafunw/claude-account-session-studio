//! Non-destructive, conflict-safe bidirectional session synchronization engine
//! for Claude Desktop (`Code` tab `local_*.json` pointers + `dframe-group-scopes`).
//!
//! Core invariants enforced:
//! 1. Source session files are NEVER deleted, moved, or modified.
//! 2. Existing target session files are NEVER overwritten in place (`copy-missing` only,
//!    or `conflict-preserve-companion` when a session diverged across accounts). The one
//!    exceptions are `metadata-refresh`: a pointer to the same chat gets only `lastActivityAt`,
//!    `completedTurns` and `title` from its newer copy, and `continue-refresh`: a chat continued
//!    under a new transcript id (old id in `priorCliSessionIds`) also gets `cliSessionId` and
//!    `priorCliSessionIds`. Both always after a verified backup.
//! 3. OAuth / server bridge tokens (`bridgeSessionIds`) are stripped (`[]`) when copying
//!    session pointers across different accounts so credentials/tokens are never mixed.
//! 4. Every sync execution requires a prior SHA-256 verified backup in the external backup
//!    directory and writes files atomically with SHA-256 verification and journal logging.

use crate::accounts::{AccountProfile, AutomationSettings};
use crate::backup::{append_journal, create_verified_backup};
use crate::dpapi::{atomic_write_verified, is_file_locked, sha256_bytes};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub session_id: String,
    pub file_name: String,
    pub file_path: String,
    pub account_uuid: String,
    pub account_label: String,
    pub org_uuid: String,
    pub cli_session_id: Option<String>,
    /// `priorCliSessionIds`: earlier transcript ids of this chat (compaction/resume); empty if missing.
    #[serde(default)]
    pub prior_cli_session_ids: Vec<String>,
    pub cwd: Option<String>,
    pub origin_cwd: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub completed_turns: i64,
    pub model: String,
    pub is_archived: bool,
    pub is_starred: bool,
    pub transcript_unavailable: bool,
    pub transcript_exists_on_disk: bool,
    pub error_notice: Option<String>,
    /// `spawnedFrom.sessionId`: the chat this one was started from (Desktop nests it there).
    pub spawned_from: Option<String>,
    /// Desktop sidebar group ("section") the chat sits in; filled by `read_sidebar_groups`.
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarGroup {
    pub id: String,
    pub name: String,
}

/// What a sync may newly copy in "ask" mode; `None` everywhere means "all" mode (every chat).
pub struct SyncSelection {
    pub chats: HashSet<String>,
    pub groups: HashSet<String>,
    /// `folder_key` values; match chats outside any sidebar group.
    pub folders: HashSet<String>,
    /// Item key -> accounts that should have it; a missing key means all accounts.
    pub targets: HashMap<String, Vec<String>>,
}

impl SyncSelection {
    pub fn from_settings(a: &AutomationSettings) -> Option<Self> {
        (a.transfer_mode == "ask").then(|| Self {
            chats: a.selected_chats.iter().cloned().collect(),
            groups: a.selected_groups.iter().cloned().collect(),
            folders: a.selected_folders.iter().map(|f| folder_key(f)).collect(),
            targets: a.selected_targets.clone(),
        })
    }

    /// Target accounts (lowercase uuids) of one picked item; `None` = all accounts.
    fn targets_of(&self, key: &str) -> Option<HashSet<String>> {
        self.targets.get(key).map(|v| v.iter().map(|u| u.to_lowercase()).collect())
    }
}

/// "ask" mode approvals: session id -> accounts that should get a new copy (`None` = all accounts).
/// A chat is approved when picked one by one, in a picked sidebar group, or (outside any group) in a
/// picked folder; several picks union their targets. Unapproved chats are missing from the map.
fn approved_targets(data_root: &Path, sessions: &[SessionRecord], sel: &SyncSelection) -> HashMap<String, Option<HashSet<String>>> {
    let (groups, membership) = read_sidebar_groups(data_root, sessions);
    let mut allowed: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    for s in sessions {
        // An assignment to a group that no longer exists counts as "no group", as in the UI.
        let group = membership.get(&s.session_id).filter(|g| groups.iter().any(|x| &x.id == *g));
        let folder = s.cwd.as_deref().or(s.origin_cwd.as_deref()).map(folder_key);
        let picks = [
            s.cli_session_id.as_ref().filter(|c| sel.chats.contains(*c)).map(|c| format!("chat:{c}")),
            group.filter(|g| sel.groups.contains(*g)).map(|g| format!("group:{g}")),
            folder.filter(|f| group.is_none() && sel.folders.contains(f)).map(|f| format!("folder:{f}")),
        ];
        for key in picks.into_iter().flatten() {
            let entry = allowed.entry(s.session_id.clone()).or_insert_with(|| Some(HashSet::new()));
            match (entry.as_mut(), sel.targets_of(&key)) {
                (Some(set), Some(t)) => set.extend(t),
                _ => *entry = None,
            }
        }
    }
    allowed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChat {
    pub cli_session_id: String,
    pub session_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub last_activity_at: i64,
    pub source_account_uuid: String,
}

/// Healthy chats `target` does not list (no pointer with that cli id) and that are not approved for it,
/// newest first, one per cli id. Empty in "all" mode (`selection` = `None`). Read-only.
pub fn pending_for_target(
    data_root: &Path,
    projects_dir: &Path,
    accounts: &[AccountProfile],
    selection: Option<&SyncSelection>,
    target: &str,
) -> Vec<PendingChat> {
    let Some(sel) = selection else { return Vec::new() };
    // Sync can never copy into an account without an org folder; asking about it would repeat forever.
    if !accounts.iter().any(|a| a.account_uuid.eq_ignore_ascii_case(target) && !a.org_uuid.is_empty()) {
        return Vec::new();
    }
    let mut sessions = scan_all_sessions(data_root, projects_dir, accounts);
    retain_listed(&mut sessions, accounts);
    let approved = approved_targets(data_root, &sessions, sel);
    let target_lc = target.to_lowercase();
    let mut listed: HashSet<&str> = HashSet::new();
    let mut approved_cli: HashSet<&str> = HashSet::new();
    for s in &sessions {
        let Some(cli) = s.cli_session_id.as_deref() else { continue };
        if s.account_uuid.eq_ignore_ascii_case(target) {
            listed.insert(cli);
            listed.extend(s.prior_cli_session_ids.iter().map(String::as_str));
        }
        if approved.get(&s.session_id).is_some_and(|t| t.as_ref().map_or(true, |t| t.contains(&target_lc))) {
            approved_cli.insert(cli);
        }
    }
    let mut newest: HashMap<&str, &SessionRecord> = HashMap::new();
    for s in &sessions {
        let Some(cli) = s.cli_session_id.as_deref() else { continue };
        // A continuation of a chat the target lists is moved along by sync, not newly copied.
        let continues_listed = s.prior_cli_session_ids.iter().any(|p| listed.contains(p.as_str()));
        if s.transcript_unavailable || listed.contains(cli) || continues_listed || approved_cli.contains(cli) {
            continue;
        }
        let e = newest.entry(cli).or_insert(s);
        if s.last_activity_at > e.last_activity_at {
            *e = s;
        }
    }
    let mut out: Vec<PendingChat> = newest
        .into_iter()
        .map(|(cli, s)| PendingChat {
            cli_session_id: cli.to_string(),
            session_id: s.session_id.clone(),
            title: s.title.clone(),
            cwd: s.cwd.clone(),
            last_activity_at: s.last_activity_at,
            source_account_uuid: s.account_uuid.clone(),
        })
        .collect();
    out.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
    out
}

/// Windows folder identity: case-insensitive, `/` and `\` alike, no trailing slash.
pub fn folder_key(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Desktop sidebar groups across all account scopes (read-only), and each session's group.
/// Only top-level chats are assigned; chats started from them inherit their parent's group.
pub fn read_sidebar_groups(data_root: &Path, sessions: &[SessionRecord]) -> (Vec<SidebarGroup>, HashMap<String, String>) {
    let mut groups: Vec<SidebarGroup> = Vec::new();
    let mut assigned: HashMap<String, String> = HashMap::new();
    let cfg = fs::read_to_string(data_root.join("claude_desktop_config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let scopes = cfg
        .as_ref()
        .and_then(|v| v.pointer("/preferences/epitaxyPrefs/dframe-group-scopes"))
        .and_then(|v| v.as_object());
    for scope in scopes.into_iter().flat_map(|s| s.values()) {
        for g in scope.get("groups").and_then(|g| g.as_array()).into_iter().flatten() {
            let id = g.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if !id.is_empty() && !groups.iter().any(|x| x.id == id) {
                let name = g.get("name").and_then(|v| v.as_str()).unwrap_or(crate::i18n::t("Bölüm", "Section"));
                groups.push(SidebarGroup { id: id.to_string(), name: name.to_string() });
            }
        }
        for (key, gid) in scope.get("assignments").and_then(|a| a.as_object()).into_iter().flatten() {
            if let Some(gid) = gid.as_str() {
                let sid = key.strip_prefix("code:").unwrap_or(key);
                assigned.entry(sid.to_string()).or_insert_with(|| gid.to_string());
            }
        }
    }
    let parent: HashMap<&str, &str> = sessions
        .iter()
        .filter_map(|s| s.spawned_from.as_deref().map(|p| (s.session_id.as_str(), p)))
        .collect();
    let mut membership = HashMap::new();
    for s in sessions {
        let mut cur = s.session_id.as_str();
        for _ in 0..32 {
            // depth cap guards against a parent loop
            if let Some(g) = assigned.get(cur) {
                membership.insert(s.session_id.clone(), g.clone());
                break;
            }
            match parent.get(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
    }
    (groups, membership)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncActionItem {
    pub session_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub cli_session_id: Option<String>,
    pub source_account_uuid: String,
    pub source_account_label: String,
    pub target_account_uuid: String,
    pub target_account_label: String,
    pub target_org_uuid: String,
    /// `"copy-new"` | `"conflict-companion"` | `"metadata-refresh"` | `"identical-skip"` | `"skip-unhealthy"`
    pub action: String,
    pub reason: String,
    pub target_file_path: String,
    /// `metadata-refresh` / `continue-refresh`: the target's `lastActivityAt` the preview saw (stale-write guard).
    #[serde(skip)]
    pub target_last_activity_at: i64,
    /// `continue-refresh` only: the target's `cliSessionId` the preview saw (stale-write guard).
    #[serde(skip)]
    pub target_cli_session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreviewReport {
    pub data_root: String,
    pub mode: String,
    pub total_accounts: usize,
    pub total_sessions_scanned: usize,
    pub copy_new_count: usize,
    pub conflict_companion_count: usize,
    pub metadata_refresh_count: usize,
    #[serde(default)]
    pub continue_refresh_count: usize,
    pub identical_skip_count: usize,
    pub unhealthy_skip_count: usize,
    pub sidebar_groups_to_merge: usize,
    pub actions: Vec<SyncActionItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncExecutionResult {
    pub pre_sync_backup_id: String,
    pub copied_new_files: usize,
    pub created_conflict_companions: usize,
    pub refreshed_metadata: usize,
    #[serde(default)]
    pub continued_pointers: usize,
    pub skipped_identical: usize,
    pub merged_sidebar_groups: usize,
    pub completed_at: String,
}

/// Build an index of all `.jsonl` transcript IDs present under `~/.claude/projects` (read-only).
pub fn index_available_cli_transcripts(projects_dir: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    if !projects_dir.is_dir() {
        return set;
    }
    if let Ok(projs) = fs::read_dir(projects_dir) {
        for proj in projs.flatten() {
            if let Ok(files) = fs::read_dir(proj.path()) {
                for f in files.flatten() {
                    let name = f.file_name().to_string_lossy().into_owned();
                    if let Some(stem) = name.strip_suffix(".jsonl") {
                        set.insert(stem.to_string());
                    }
                }
            }
        }
    }
    set
}

/// Drops chats of account folders not in `accounts` (hidden accounts): they are no sync source.
fn retain_listed(sessions: &mut Vec<SessionRecord>, accounts: &[AccountProfile]) {
    sessions.retain(|s| accounts.iter().any(|a| a.account_uuid.eq_ignore_ascii_case(&s.account_uuid)));
}

/// `priorCliSessionIds` as strings; missing, non-array or non-string entries are ignored.
fn prior_cli_ids(val: &Value) -> Vec<String> {
    val.get("priorCliSessionIds")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Scan all `local_*.json` session pointers across all accounts in `data_root`.
pub fn scan_all_sessions(
    data_root: &Path,
    projects_dir: &Path,
    accounts: &[AccountProfile],
) -> Vec<SessionRecord> {
    let transcript_ids = index_available_cli_transcripts(projects_dir);
    let label_by_uuid: HashMap<String, String> = accounts
        .iter()
        .map(|a| (a.account_uuid.clone(), a.label.clone()))
        .collect();

    let sessions_root = data_root.join("claude-code-sessions");
    let mut records = Vec::new();

    let Ok(acc_dirs) = fs::read_dir(&sessions_root) else {
        return records;
    };

    for acc_entry in acc_dirs.flatten() {
        if !acc_entry.path().is_dir() {
            continue;
        }
        let account_uuid = acc_entry.file_name().to_string_lossy().into_owned();
        let account_label = label_by_uuid
            .get(&account_uuid)
            .cloned()
            .unwrap_or_else(|| format!("Account {}", &account_uuid[..8.min(account_uuid.len())]));

        let Ok(org_dirs) = fs::read_dir(acc_entry.path()) else {
            continue;
        };

        for org_entry in org_dirs.flatten() {
            if !org_entry.path().is_dir() {
                continue;
            }
            let org_uuid = org_entry.file_name().to_string_lossy().into_owned();
            let Ok(files) = fs::read_dir(org_entry.path()) else {
                continue;
            };

            for f in files.flatten() {
                let fname = f.file_name().to_string_lossy().into_owned();
                if !fname.starts_with("local_") || !fname.ends_with(".json") {
                    continue;
                }
                let fpath = f.path();
                let Ok(raw) = fs::read_to_string(&fpath) else {
                    continue;
                };
                let Ok(val) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };

                let session_id = val
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| fname.trim_end_matches(".json").to_string());

                let cli_session_id = val
                    .get("cliSessionId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let transcript_exists = cli_session_id
                    .as_ref()
                    .map(|id| transcript_ids.contains(id))
                    .unwrap_or(false);

                let cwd = val
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let origin_cwd = val
                    .get("originCwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let title = val
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled Code Session")
                    .to_string();
                let created_at = val.get("createdAt").and_then(|v| v.as_i64()).unwrap_or(0);
                let last_activity_at = val
                    .get("lastActivityAt")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(created_at);
                let completed_turns = val
                    .get("completedTurns")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let model = val
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("claude-sonnet-4-6")
                    .to_string();
                let is_archived = val
                    .get("isArchived")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let is_starred = val
                    .get("isStarred")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let transcript_unavailable = val
                    .get("transcriptUnavailable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let error_notice = val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let spawned_from = val
                    .pointer("/spawnedFrom/sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                records.push(SessionRecord {
                    session_id,
                    file_name: fname,
                    file_path: fpath.to_string_lossy().into_owned(),
                    account_uuid: account_uuid.clone(),
                    account_label: account_label.clone(),
                    org_uuid: org_uuid.clone(),
                    cli_session_id,
                    prior_cli_session_ids: prior_cli_ids(&val),
                    cwd,
                    origin_cwd,
                    title,
                    created_at,
                    last_activity_at,
                    completed_turns,
                    model,
                    is_archived,
                    is_starred,
                    transcript_unavailable,
                    transcript_exists_on_disk: transcript_exists,
                    error_notice,
                    spawned_from,
                    group_id: None,
                });
            }
        }
    }

    records.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
    records
}

/// Sanitize a session JSON payload for cross-account transfer:
/// - Strips `bridgeSessionIds` to `[]` so account-specific server bridge tokens are never leaked.
/// - Clears transient rate-limit `error` banners from the source account so the target account
///   can immediately continue the session without stale rate-limit blocks.
/// - Preserves `sessionId`, `cliSessionId`, `cwd`, `originCwd`, `title`, `createdAt`,
///   `lastActivityAt`, `completedTurns`, `model`, `isArchived`, `isStarred`, `rewindEdges`, etc.
fn prepare_cross_account_session_json(
    raw_source: &str,
    override_session_id: Option<&str>,
    override_title: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut val: Value =
        serde_json::from_str(raw_source).map_err(|e| format!("Invalid session JSON: {e}"))?;

    if let Some(obj) = val.as_object_mut() {
        // Never copy account-bound bridgeSessionIds to another account
        obj.insert("bridgeSessionIds".to_string(), json!([]));
        // Clear account-specific rate-limit error notice on target copy
        obj.remove("error");

        if let Some(new_id) = override_session_id {
            obj.insert("sessionId".to_string(), json!(new_id));
        }
        if let Some(new_title) = override_title {
            obj.insert("title".to_string(), json!(new_title));
        }
    }

    serde_json::to_vec_pretty(&val).map_err(|e| format!("Failed serializing session JSON: {e}"))
}

/// Deterministic companion session ID when a session has diverged between two accounts,
/// ensuring zero overwrites of existing files while preserving the newer continuation.
fn make_companion_session_id(base_session_id: &str, source_acc_uuid: &str, cli_sid: &str, turns: i64) -> String {
    let seed = format!("{base_session_id}:{source_acc_uuid}:{cli_sid}:{turns}");
    let digest = sha256_bytes(seed.as_bytes());
    // Format as standard UUID-like suffix: 8-4-4-4-12 from SHA-256 hex
    let u = format!(
        "{}-{}-{}-{}-{}",
        &digest[0..8],
        &digest[8..12],
        &digest[12..16],
        &digest[16..20],
        &digest[20..32]
    );
    format!("local_{u}")
}

/// Count how many sidebar groups in `claude_desktop_config.json` -> `dframe-group-scopes`
/// can be additively merged across account scopes.
fn preview_sidebar_groups_merge(data_root: &Path, accounts: &[AccountProfile]) -> usize {
    let cfg_path = data_root.join("claude_desktop_config.json");
    let Ok(raw) = fs::read_to_string(&cfg_path) else {
        return 0;
    };
    let Ok(val) = serde_json::from_str::<Value>(&raw) else {
        return 0;
    };
    let Some(scopes) = val
        .pointer("/preferences/epitaxyPrefs/dframe-group-scopes")
        .and_then(|v| v.as_object())
    else {
        return 0;
    };

    let mut all_group_ids: HashSet<String> = HashSet::new();
    for (_, scope_val) in scopes {
        if let Some(groups) = scope_val.get("groups").and_then(|g| g.as_array()) {
            for g in groups {
                if let Some(id) = g.get("id").and_then(|v| v.as_str()) {
                    all_group_ids.insert(id.to_string());
                }
            }
        }
    }
    if all_group_ids.is_empty() {
        return 0;
    }

    let mut missing_total = 0usize;
    for acc in accounts {
        if acc.org_uuid.is_empty() {
            continue;
        }
        let key = format!("{}/{}", acc.account_uuid, acc.org_uuid);
        let existing_count = scopes
            .get(&key)
            .and_then(|s| s.get("groups"))
            .and_then(|g| g.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if all_group_ids.len() > existing_count {
            missing_total += all_group_ids.len() - existing_count;
        }
    }
    missing_total
}

/// Additively merge `dframe-group-scopes` in `claude_desktop_config.json` across all account scopes.
/// Never deletes any group or assignment.
fn execute_sidebar_groups_merge(data_root: &Path, accounts: &[AccountProfile]) -> Result<usize, String> {
    let cfg_path = data_root.join("claude_desktop_config.json");
    if !cfg_path.is_file() {
        return Ok(0);
    }
    let raw = fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
    let mut root_val: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let Some(scopes_val) = root_val.pointer_mut("/preferences/epitaxyPrefs/dframe-group-scopes") else {
        return Ok(0);
    };
    let Some(scopes_map) = scopes_val.as_object_mut() else {
        return Ok(0);
    };

    // Collect union of all groups and assignments across scopes
    let mut union_groups: Vec<Value> = Vec::new();
    let mut seen_group_ids: HashSet<String> = HashSet::new();
    let mut union_assignments: serde_json::Map<String, Value> = serde_json::Map::new();

    for (_, scope_obj) in scopes_map.iter() {
        if let Some(groups) = scope_obj.get("groups").and_then(|g| g.as_array()) {
            for g in groups {
                if let Some(id) = g.get("id").and_then(|v| v.as_str()) {
                    if seen_group_ids.insert(id.to_string()) {
                        union_groups.push(g.clone());
                    }
                }
            }
        }
        if let Some(assigns) = scope_obj.get("assignments").and_then(|a| a.as_object()) {
            for (sid, gid) in assigns {
                union_assignments
                    .entry(sid.clone())
                    .or_insert_with(|| gid.clone());
            }
        }
    }

    if union_groups.is_empty() && union_assignments.is_empty() {
        return Ok(0);
    }

    let mut added_count = 0usize;
    for acc in accounts {
        if acc.org_uuid.is_empty() {
            continue;
        }
        let scope_key = format!("{}/{}", acc.account_uuid, acc.org_uuid);
        let entry = scopes_map.entry(scope_key).or_insert_with(|| {
            json!({
                "groups": [],
                "assignments": {},
                "order": {}
            })
        });

        if let Some(scope_obj) = entry.as_object_mut() {
            let groups_arr = scope_obj
                .entry("groups")
                .or_insert_with(|| json!([]))
                .as_array_mut();
            if let Some(arr) = groups_arr {
                let existing_ids: HashSet<String> = arr
                    .iter()
                    .filter_map(|g| g.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect();
                for ug in &union_groups {
                    if let Some(id) = ug.get("id").and_then(|v| v.as_str()) {
                        if !existing_ids.contains(id) {
                            arr.push(ug.clone());
                            added_count += 1;
                        }
                    }
                }
            }

            let assigns_obj = scope_obj
                .entry("assignments")
                .or_insert_with(|| json!({}))
                .as_object_mut();
            if let Some(a_map) = assigns_obj {
                for (sid, gid) in &union_assignments {
                    a_map.entry(sid.clone()).or_insert_with(|| gid.clone());
                }
            }
        }
    }

    if added_count > 0 {
        let bytes = serde_json::to_vec_pretty(&root_val).map_err(|e| e.to_string())?;
        atomic_write_verified(&cfg_path, &bytes)?;
    }

    Ok(added_count)
}

/// Generate a complete, read-only preview of all session synchronizations across accounts.
/// Never modifies any file on disk.
pub fn preview_sync(
    data_root: &Path,
    projects_dir: &Path,
    accounts: &[AccountProfile],
    mode: &str,
    selection: Option<&SyncSelection>,
) -> SyncPreviewReport {
    let mut all_sessions = scan_all_sessions(data_root, projects_dir, accounts);
    retain_listed(&mut all_sessions, accounts);

    // "ask" mode: unapproved chats and non-target accounts get no NEW copy; a chat an account already has
    // keeps its refresh/companion there either way. Nothing is ever removed.
    let approved = selection.map(|sel| approved_targets(data_root, &all_sessions, sel));

    // Map (account_uuid, session_id) -> SessionRecord
    let mut by_acc_and_sid: HashMap<(String, String), SessionRecord> = HashMap::new();
    for s in &all_sessions {
        by_acc_and_sid.insert((s.account_uuid.clone(), s.session_id.clone()), s.clone());
    }

    // Group all unique session IDs and find the healthiest / newest version of each session
    let mut unique_sids: Vec<String> = all_sessions
        .iter()
        .map(|s| s.session_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique_sids.sort();

    // (account, CLI transcript id) pairs each account already lists, plus those planned in this run.
    // The transcript `.jsonl` is shared by all accounts, so a second pointer to it is a duplicate chat.
    // A pointer's `priorCliSessionIds` count too: an account that moved on from a transcript must not
    // get the older state back as another chat (e.g. a "[Continued in …]" companion from older versions).
    let mut transcript_in_acc: HashSet<(String, String)> = all_sessions
        .iter()
        .filter(|s| !s.transcript_unavailable)
        .flat_map(|s| {
            s.cli_session_id
                .iter()
                .chain(&s.prior_cli_session_ids)
                .map(|c| (s.account_uuid.clone(), c.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    let mut actions: Vec<SyncActionItem> = Vec::new();
    let mut copy_new_count = 0usize;
    let mut conflict_companion_count = 0usize;
    let mut metadata_refresh_count = 0usize;
    let mut continue_refresh_count = 0usize;
    let mut identical_skip_count = 0usize;
    let mut unhealthy_skip_count = 0usize;

    for sid in &unique_sids {
        // `None` = not approved anywhere, `Some(None)` = approved for all accounts.
        let approval: Option<Option<&HashSet<String>>> = match approved.as_ref() {
            Some(a) => a.get(sid).map(|t| t.as_ref()),
            None => Some(None),
        };
        // Find all copies of this session across accounts
        let copies: Vec<&SessionRecord> = all_sessions
            .iter()
            .filter(|s| &s.session_id == sid)
            .collect();
        if copies.is_empty() {
            continue;
        }

        // Pick canonical best source: prefer healthy (`!transcript_unavailable`), highest `completed_turns`, newest `last_activity_at`
        let best_source = copies
            .iter()
            .max_by(|a, b| {
                (!a.transcript_unavailable)
                    .cmp(&(!b.transcript_unavailable))
                    .then_with(|| a.completed_turns.cmp(&b.completed_turns))
                    .then_with(|| a.last_activity_at.cmp(&b.last_activity_at))
            })
            .copied()
            .unwrap();

        for target_acc in accounts {
            if target_acc
                .account_uuid
                .eq_ignore_ascii_case(&best_source.account_uuid)
            {
                continue;
            }
            if target_acc.org_uuid.is_empty() {
                continue;
            }
            let may_copy = approval.is_some_and(|t| t.map_or(true, |t| t.contains(&target_acc.account_uuid.to_lowercase())));

            let target_dir = data_root
                .join("claude-code-sessions")
                .join(&target_acc.account_uuid)
                .join(&target_acc.org_uuid);

            if best_source.transcript_unavailable || best_source.cli_session_id.is_none() {
                if !may_copy {
                    continue;
                }
                unhealthy_skip_count += 1;
                actions.push(SyncActionItem {
                    session_id: best_source.session_id.clone(),
                    title: best_source.title.clone(),
                    cwd: best_source.cwd.clone(),
                    cli_session_id: best_source.cli_session_id.clone(),
                    source_account_uuid: best_source.account_uuid.clone(),
                    source_account_label: best_source.account_label.clone(),
                    target_account_uuid: target_acc.account_uuid.clone(),
                    target_account_label: target_acc.label.clone(),
                    target_org_uuid: target_acc.org_uuid.clone(),
                    action: "skip-unhealthy".to_string(),
                    reason: "Source session has no valid cliSessionId or transcriptUnavailable=true"
                        .to_string(),
                    target_file_path: target_dir
                        .join(&best_source.file_name)
                        .to_string_lossy()
                        .into_owned(),
                    target_last_activity_at: 0,
                    target_cli_session_id: None,
                });
                continue;
            }

            let transcript_key = (
                target_acc.account_uuid.clone(),
                best_source.cli_session_id.clone().unwrap_or_default(),
            );
            // Same pointer, but the source continued the chat under a new transcript id (compaction /
            // resume lists the target's id in `priorCliSessionIds`): move the target's pointer along
            // instead of adding a "[Continued in …]" companion. Also when an old companion already lists
            // the new id, so the original pointer catches up.
            let continued = by_acc_and_sid
                .get(&(target_acc.account_uuid.clone(), sid.clone()))
                .filter(|t| {
                    !t.transcript_unavailable
                        && t.cli_session_id != best_source.cli_session_id
                        && t.cli_session_id.as_ref().is_some_and(|c| best_source.prior_cli_session_ids.contains(c))
                        && best_source.last_activity_at > t.last_activity_at
                });
            if let Some(t) = continued {
                continue_refresh_count += 1;
                transcript_in_acc.insert(transcript_key);
                actions.push(SyncActionItem {
                    session_id: best_source.session_id.clone(),
                    title: best_source.title.clone(),
                    cwd: best_source.cwd.clone(),
                    cli_session_id: best_source.cli_session_id.clone(),
                    source_account_uuid: best_source.account_uuid.clone(),
                    source_account_label: best_source.account_label.clone(),
                    target_account_uuid: target_acc.account_uuid.clone(),
                    target_account_label: target_acc.label.clone(),
                    target_org_uuid: target_acc.org_uuid.clone(),
                    action: "continue-refresh".to_string(),
                    reason: format!(
                        "Continued in '{}' under a new transcript id: the existing pointer is updated in place ({} -> {} turns)",
                        best_source.account_label, t.completed_turns, best_source.completed_turns
                    ),
                    target_file_path: t.file_path.clone(),
                    target_last_activity_at: t.last_activity_at,
                    target_cli_session_id: t.cli_session_id.clone(),
                });
                continue;
            }
            if transcript_in_acc.contains(&transcript_key) {
                // Target already lists this exact chat (maybe under another session id): never add it twice.
                // If it is the same pointer with an older date, only its date/turns/title get refreshed.
                let stale = by_acc_and_sid
                    .get(&(target_acc.account_uuid.clone(), sid.clone()))
                    .filter(|t| {
                        t.cli_session_id == best_source.cli_session_id
                            && !t.transcript_unavailable
                            && best_source.last_activity_at > t.last_activity_at
                    });
                if let Some(t) = stale {
                    metadata_refresh_count += 1;
                    actions.push(SyncActionItem {
                        session_id: best_source.session_id.clone(),
                        title: best_source.title.clone(),
                        cwd: best_source.cwd.clone(),
                        cli_session_id: best_source.cli_session_id.clone(),
                        source_account_uuid: best_source.account_uuid.clone(),
                        source_account_label: best_source.account_label.clone(),
                        target_account_uuid: target_acc.account_uuid.clone(),
                        target_account_label: target_acc.label.clone(),
                        target_org_uuid: target_acc.org_uuid.clone(),
                        action: "metadata-refresh".to_string(),
                        reason: format!(
                            "Continued in '{}': only lastActivityAt/completedTurns/title will be updated ({} -> {} turns)",
                            best_source.account_label, t.completed_turns, best_source.completed_turns
                        ),
                        target_file_path: t.file_path.clone(),
                        target_last_activity_at: t.last_activity_at,
                        target_cli_session_id: t.cli_session_id.clone(),
                    });
                    continue;
                }
                identical_skip_count += 1;
                actions.push(SyncActionItem {
                    session_id: best_source.session_id.clone(),
                    title: best_source.title.clone(),
                    cwd: best_source.cwd.clone(),
                    cli_session_id: best_source.cli_session_id.clone(),
                    source_account_uuid: best_source.account_uuid.clone(),
                    source_account_label: best_source.account_label.clone(),
                    target_account_uuid: target_acc.account_uuid.clone(),
                    target_account_label: target_acc.label.clone(),
                    target_org_uuid: target_acc.org_uuid.clone(),
                    action: "identical-skip".to_string(),
                    reason: "Same chat transcript already present in target account (existing file untouched)"
                        .to_string(),
                    target_file_path: target_dir
                        .join(&best_source.file_name)
                        .to_string_lossy()
                        .into_owned(),
                    target_last_activity_at: 0,
                    target_cli_session_id: None,
                });
                continue;
            }
            let target_has_sid = by_acc_and_sid.contains_key(&(target_acc.account_uuid.clone(), sid.clone()));
            if !may_copy && !target_has_sid {
                continue;
            }
            transcript_in_acc.insert(transcript_key);

            match by_acc_and_sid.get(&(target_acc.account_uuid.clone(), sid.clone())) {
                None => {
                    // Target account does not have this session at all -> safe additive creation!
                    copy_new_count += 1;
                    actions.push(SyncActionItem {
                        session_id: best_source.session_id.clone(),
                        title: best_source.title.clone(),
                        cwd: best_source.cwd.clone(),
                        cli_session_id: best_source.cli_session_id.clone(),
                        source_account_uuid: best_source.account_uuid.clone(),
                        source_account_label: best_source.account_label.clone(),
                        target_account_uuid: target_acc.account_uuid.clone(),
                        target_account_label: target_acc.label.clone(),
                        target_org_uuid: target_acc.org_uuid.clone(),
                        action: "copy-new".to_string(),
                        reason: format!(
                            "Missing in '{}' -> will create pointer to CLI transcript ({} turns)",
                            target_acc.label, best_source.completed_turns
                        ),
                        target_file_path: target_dir
                            .join(&best_source.file_name)
                            .to_string_lossy()
                            .into_owned(),
                        target_last_activity_at: 0,
                        target_cli_session_id: None,
                    });
                }
                Some(existing_target) => {
                    // Target ALREADY has `local_<id>.json` but it points to a different (or unavailable)
                    // transcript. NEVER overwrite it; add a companion pointer instead.
                    let comp_sid = make_companion_session_id(
                        &best_source.session_id,
                        &best_source.account_uuid,
                        best_source.cli_session_id.as_deref().unwrap_or(""),
                        best_source.completed_turns,
                    );
                    let comp_file = target_dir.join(format!("{comp_sid}.json"));
                    conflict_companion_count += 1;
                    actions.push(SyncActionItem {
                        session_id: comp_sid.clone(),
                        title: format!(
                            "{} [Continued in {}]",
                            best_source.title, best_source.account_label
                        ),
                        cwd: best_source.cwd.clone(),
                        cli_session_id: best_source.cli_session_id.clone(),
                        source_account_uuid: best_source.account_uuid.clone(),
                        source_account_label: best_source.account_label.clone(),
                        target_account_uuid: target_acc.account_uuid.clone(),
                        target_account_label: target_acc.label.clone(),
                        target_org_uuid: target_acc.org_uuid.clone(),
                        action: "conflict-companion".to_string(),
                        reason: format!(
                            "Conflict resolved without overwrite: keeps existing {} intact & creates companion pointer for newer continuation ({} -> {} turns)",
                            existing_target.session_id,
                            existing_target.completed_turns,
                            best_source.completed_turns
                        ),
                        target_file_path: comp_file.to_string_lossy().into_owned(),
                        target_last_activity_at: 0,
                        target_cli_session_id: None,
                    });
                }
            }
        }
    }

    // Sort actions so actionable ones (`copy-new`, `conflict-companion`) appear first
    actions.sort_by(|a, b| {
        let rank = |act: &str| match act {
            "copy-new" => 0,
            "conflict-companion" | "metadata-refresh" | "continue-refresh" => 1,
            "skip-unhealthy" => 2,
            _ => 3,
        };
        rank(&a.action)
            .cmp(&rank(&b.action))
            .then_with(|| a.title.cmp(&b.title))
    });

    let sidebar_groups_to_merge = preview_sidebar_groups_merge(data_root, accounts);

    SyncPreviewReport {
        data_root: data_root.to_string_lossy().into_owned(),
        mode: mode.to_string(),
        total_accounts: accounts.len(),
        total_sessions_scanned: all_sessions.len(),
        copy_new_count,
        conflict_companion_count,
        metadata_refresh_count,
        continue_refresh_count,
        identical_skip_count,
        unhealthy_skip_count,
        sidebar_groups_to_merge,
        actions,
    }
}

/// The only keys a `metadata-refresh` copies into an existing pointer.
const METADATA_KEYS: [&str; 3] = ["lastActivityAt", "completedTurns", "title"];

/// The only keys a `continue-refresh` copies: the chat's transcript ids plus `METADATA_KEYS`.
const CONTINUE_KEYS: [&str; 5] = ["cliSessionId", "priorCliSessionIds", "lastActivityAt", "completedTurns", "title"];

/// `target_raw` with only `METADATA_KEYS` (or `CONTINUE_KEYS` when `continued_from` is the target's
/// cli id the preview saw) taken from `source`; every other key keeps its value.
/// `None` when the target is not a JSON object, changed since the preview (`lastActivityAt` differs
/// from `expected_last_activity`), points to another transcript than expected, or the source is not newer.
/// A continuation also needs the target's cli id in the source's `priorCliSessionIds`.
fn refresh_metadata_json(
    target_raw: &str,
    source: &Value,
    expected_last_activity: i64,
    continued_from: Option<&str>,
) -> Option<Vec<u8>> {
    let mut target: Value = serde_json::from_str(target_raw).ok()?;
    let obj = target.as_object_mut()?;
    let target_cli = obj.get("cliSessionId").and_then(|v| v.as_str());
    let source_cli = source.get("cliSessionId").and_then(|v| v.as_str());
    let source_last = source.get("lastActivityAt").and_then(|v| v.as_i64());
    let same_chat = match continued_from {
        None => target_cli == source_cli,
        Some(expected_cli) => {
            target_cli == Some(expected_cli)
                && source_cli.is_some_and(|s| !s.is_empty() && s != expected_cli)
                && prior_cli_ids(source).iter().any(|p| p == expected_cli)
        }
    };
    if obj.get("lastActivityAt").and_then(|v| v.as_i64()) != Some(expected_last_activity)
        || target_cli.is_none()
        || !same_chat
        || !source_last.is_some_and(|s| s > expected_last_activity)
    {
        return None;
    }
    let keys: &[&str] = if continued_from.is_some() { &CONTINUE_KEYS } else { &METADATA_KEYS };
    for &key in keys {
        if let Some(v) = source.get(key) {
            obj.insert(key.to_string(), v.clone());
        }
    }
    serde_json::to_vec_pretty(&target).ok()
}

/// New bytes for a staged `metadata-refresh` / `continue-refresh` item, or `None` to leave the target alone.
fn refreshed_bytes(item: &SyncActionItem, target_raw: &str, source: &Value) -> Option<Vec<u8>> {
    let continued_from = if item.action == "continue-refresh" {
        Some(item.target_cli_session_id.as_deref()?)
    } else {
        None
    };
    refresh_metadata_json(target_raw, source, item.target_last_activity_at, continued_from)
}

/// Execute the non-destructive session synchronization across accounts.
/// - Creates a SHA-256 verified backup in `backup_storage_dir` BEFORE any write.
/// - Never deletes or moves a session file; an existing one is only touched by
///   `metadata-refresh` (date/turns/title) or `continue-refresh` (plus transcript ids), and only
///   while Claude Desktop is not running.
/// - Strips `bridgeSessionIds` (`[]`) so OAuth/bridge tokens never cross accounts.
/// - Checks for file locks and aborts cleanly if any unexpected error occurs.
pub fn execute_sync(
    data_root: &Path,
    projects_dir: &Path,
    backup_storage_dir: &Path,
    accounts: &[AccountProfile],
    sync_sidebar_groups: bool,
    mode: &str,
    take_backup: bool,
    selection: Option<&SyncSelection>,
) -> Result<SyncExecutionResult, String> {
    // A running Desktop rewrites the signed-in account's pointers, and every refresh forces a full
    // backup while the active chat keeps the others stale; refresh once Desktop is closed instead.
    let allow_metadata_refresh = !crate::paths::is_claude_desktop_running(Some(data_root));
    execute_sync_inner(
        data_root,
        projects_dir,
        backup_storage_dir,
        accounts,
        sync_sidebar_groups,
        mode,
        take_backup,
        allow_metadata_refresh,
        selection,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_sync_inner(
    data_root: &Path,
    projects_dir: &Path,
    backup_storage_dir: &Path,
    accounts: &[AccountProfile],
    sync_sidebar_groups: bool,
    mode: &str,
    take_backup: bool,
    allow_metadata_refresh: bool,
    selection: Option<&SyncSelection>,
) -> Result<SyncExecutionResult, String> {
    if let Some(detail) = crate::format_guard::check(data_root) {
        let msg = format!("{} {detail}", crate::format_guard::stopped());
        // One entry per problem, not one per background tick.
        let last = crate::backup::read_journal(backup_storage_dir, 1);
        if !last.first().is_some_and(|j| j.operation == "format-guard" && j.details == msg) {
            append_journal(backup_storage_dir, "format-guard", mode, &msg, "error");
        }
        return Err(msg);
    }
    let preview = preview_sync(data_root, projects_dir, accounts, mode, selection);

    // 1. Pre-flight check: verify no target path already exists or is locked
    let mut staged_writes: Vec<(PathBuf, Vec<u8>, String)> = Vec::new();
    let mut staged_refreshes: Vec<(&SyncActionItem, Value)> = Vec::new();
    let all_sessions = scan_all_sessions(data_root, projects_dir, accounts);
    let source_by_acc_and_title: HashMap<(String, String), &SessionRecord> = all_sessions
        .iter()
        .map(|s| ((s.account_uuid.clone(), s.session_id.clone()), s))
        .collect();

    for item in &preview.actions {
        if item.action == "metadata-refresh" || item.action == "continue-refresh" {
            if !allow_metadata_refresh {
                continue;
            }
            let target_path = Path::new(&item.target_file_path);
            let source = source_by_acc_and_title
                .get(&(item.source_account_uuid.clone(), item.session_id.clone()))
                .and_then(|s| fs::read_to_string(&s.file_path).ok())
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            // Stage only what would really be written, so an unwritable target never forces a backup.
            if let Some(source) = source {
                let writable = !is_file_locked(target_path)
                    && fs::read_to_string(target_path)
                        .ok()
                        .and_then(|raw| refreshed_bytes(item, &raw, &source))
                        .is_some();
                if writable {
                    staged_refreshes.push((item, source));
                }
            }
            continue;
        }
        if item.action != "copy-new" && item.action != "conflict-companion" {
            continue;
        }
        let target_path = PathBuf::from(&item.target_file_path);

        // Double-check that target file does NOT exist so we NEVER overwrite
        if target_path.exists() {
            if is_file_locked(&target_path) {
                return Err(format!(
                    "Sync aborted: Target file {} is locked by another process.",
                    target_path.display()
                ));
            }
            continue;
        }

        // Locate original source session file
        let src_record = if item.action == "copy-new" {
            source_by_acc_and_title
                .get(&(item.source_account_uuid.clone(), item.session_id.clone()))
                .copied()
        } else {
            // For conflict-companion, find by source_account_uuid + cli_session_id
            all_sessions.iter().find(|s| {
                s.account_uuid == item.source_account_uuid
                    && s.cli_session_id == item.cli_session_id
            })
        };

        let Some(src_rec) = src_record else {
            return Err(format!(
                "Sync aborted: Could not locate source session record for {}",
                item.session_id
            ));
        };

        let raw_source = fs::read_to_string(&src_rec.file_path).map_err(|e| {
            format!(
                "Sync aborted: Failed reading source session {}: {e}",
                src_rec.file_path
            )
        })?;

        let prepared_bytes = if item.action == "conflict-companion" {
            prepare_cross_account_session_json(
                &raw_source,
                Some(&item.session_id),
                Some(&item.title),
            )?
        } else {
            prepare_cross_account_session_json(&raw_source, None, None)?
        };

        staged_writes.push((target_path, prepared_bytes, item.action.clone()));
    }

    // Nothing would be written: skip the (large) backup so background syncs stay cheap.
    if staged_writes.is_empty() && staged_refreshes.is_empty() && !sync_sidebar_groups {
        return Ok(SyncExecutionResult {
            pre_sync_backup_id: String::new(),
            copied_new_files: 0,
            created_conflict_companions: 0,
            refreshed_metadata: 0,
            continued_pointers: 0,
            skipped_identical: preview.identical_skip_count,
            merged_sidebar_groups: 0,
            completed_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    // 2. SHA-256 verified backup before writing. Callers may skip it only for pure additions
    //    (no sidebar merge, no metadata refresh), where no existing file is touched; the
    //    background loop throttles it.
    let backup_id = if take_backup || sync_sidebar_groups || !staged_refreshes.is_empty() {
        create_verified_backup(
            data_root,
            backup_storage_dir,
            &format!(
                "Pre-sync verified snapshot ({} new, {} conflict-companions, {} metadata refreshes, {} continued pointers)",
                preview.copy_new_count,
                preview.conflict_companion_count,
                staged_refreshes.iter().filter(|(i, _)| i.action == "metadata-refresh").count(),
                staged_refreshes.iter().filter(|(i, _)| i.action == "continue-refresh").count()
            ),
            mode,
        )?
        .id
    } else {
        "none (additions only)".to_string()
    };

    // 3. Execute atomic verified writes
    let mut copied_new_files = 0usize;
    let mut created_conflict_companions = 0usize;

    for (target_path, bytes, action_kind) in staged_writes {
        if target_path.exists() {
            // Strict safety guard: never overwrite an existing file
            continue;
        }
        atomic_write_verified(&target_path, &bytes)?;
        if action_kind == "copy-new" {
            copied_new_files += 1;
        } else {
            created_conflict_companions += 1;
        }
    }

    let mut refreshed_metadata = 0usize;
    let mut continued_pointers = 0usize;
    for (item, source) in staged_refreshes {
        let target_path = Path::new(&item.target_file_path);
        // Re-read right before writing: Desktop may have rewritten it while the backup ran.
        let Some(bytes) = fs::read_to_string(target_path)
            .ok()
            .and_then(|raw| refreshed_bytes(item, &raw, &source))
        else {
            continue;
        };
        if is_file_locked(target_path) {
            continue;
        }
        atomic_write_verified(target_path, &bytes)?;
        if item.action == "continue-refresh" {
            continued_pointers += 1;
        } else {
            refreshed_metadata += 1;
        }
    }

    // 4. Additively merge sidebar groups (`dframe-group-scopes`) if enabled
    let merged_sidebar_groups = if sync_sidebar_groups {
        execute_sidebar_groups_merge(data_root, accounts).unwrap_or(0)
    } else {
        0
    };

    let completed_at = chrono::Utc::now().to_rfc3339();
    append_journal(
        backup_storage_dir,
        "session-sync",
        mode,
        &format!(
            "Synced across {} accounts: {} new copied, {} conflict companions created, {} metadata refreshed, {} continued pointers updated, {} identical skipped, {} sidebar groups merged [Backup: {}]",
            accounts.len(),
            copied_new_files,
            created_conflict_companions,
            refreshed_metadata,
            continued_pointers,
            preview.identical_skip_count,
            merged_sidebar_groups,
            backup_id
        ),
        "ok",
    );

    Ok(SyncExecutionResult {
        pre_sync_backup_id: backup_id,
        copied_new_files,
        created_conflict_companions,
        refreshed_metadata,
        continued_pointers,
        skipped_identical: preview.identical_skip_count,
        merged_sidebar_groups,
        completed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountProfile, UsageMetrics};
    use crate::dpapi::sha256_file;

    /// What Claude Desktop does on an account switch: sets `lastKnownAccountUuid`, keeps other keys.
    fn set_active_account(data_root: &Path, acc: &AccountProfile) {
        let path = data_root.join("config.json");
        let mut cfg: Value = fs::read_to_string(&path)
            .ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_else(|| json!({}));
        cfg["lastKnownAccountUuid"] = json!(acc.account_uuid);
        fs::write(&path, serde_json::to_vec_pretty(&cfg).unwrap()).unwrap();
    }

    fn sample_account(uuid: &str, org: &str, label: &str, active: bool) -> AccountProfile {
        AccountProfile {
            id: format!("id-{label}"),
            label: label.to_string(),
            custom_label: None,
            email: format!("{label}@example.com"),
            account_uuid: uuid.to_string(),
            org_uuid: org.to_string(),
            plan_type: "Claude Max 20x".to_string(),
            rate_limit_tier: "default_claude_max_20x".to_string(),
            has_dpapi_credentials: true,
            is_active: active,
            session_count: 0,
            healthy_session_count: 0,
            last_activity_at: None,
            usage: UsageMetrics::default(),
            added_at: "2026-11-09T23:00:00Z".to_string(),
        }
    }

    #[test]
    fn only_selected_chats_and_group_members_are_copied() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects").join("proj");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false);
        let accounts = vec![acc_a.clone(), acc_b.clone()];
        let dir_a = data_root.join("claude-code-sessions").join(&acc_a.account_uuid).join(&acc_a.org_uuid);
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(data_root.join("claude-code-sessions").join(&acc_b.account_uuid).join(&acc_b.org_uuid)).unwrap();
        fs::create_dir_all(&projects_dir).unwrap();
        // picked: chat by id; grouped root + its child via spawnedFrom; other: nothing
        for (sid, parent) in [("picked", None), ("grouped", None), ("child", Some("local_grouped")), ("other", None)] {
            fs::write(projects_dir.join(format!("cli-{sid}.jsonl")), "{}").unwrap();
            let mut v = json!({"sessionId": format!("local_{sid}"), "cliSessionId": format!("cli-{sid}"), "title": sid, "createdAt": 1, "lastActivityAt": 2});
            if let Some(p) = parent {
                v["spawnedFrom"] = json!({"sessionId": p});
            }
            fs::write(dir_a.join(format!("local_{sid}.json")), v.to_string()).unwrap();
        }
        let scope = format!("{}/{}", acc_a.account_uuid, acc_a.org_uuid);
        let cfg = json!({"preferences": {"epitaxyPrefs": {"dframe-group-scopes": {scope: {
            "groups": [{"id": "cg-1", "name": "Bölüm"}], "assignments": {"code:local_grouped": "cg-1"}, "order": {}
        }}}}});
        fs::write(data_root.join("claude_desktop_config.json"), cfg.to_string()).unwrap();

        let sel = SyncSelection {
            chats: ["cli-picked".to_string()].into(),
            groups: ["cg-1".to_string()].into(),
            folders: HashSet::new(),
            targets: HashMap::new(),
        };
        let preview = preview_sync(&data_root, projects_dir.parent().unwrap(), &accounts, "sandbox", Some(&sel));
        let mut copied: Vec<&str> = preview.actions.iter().filter(|a| a.action == "copy-new").map(|a| a.title.as_str()).collect();
        copied.sort();
        assert_eq!(copied, ["child", "grouped", "picked"]);
        // "all" mode still copies everything
        assert_eq!(preview_sync(&data_root, projects_dir.parent().unwrap(), &accounts, "sandbox", None).copy_new_count, 4);
    }

    #[test]
    fn folder_pick_copies_its_chats_and_targets_limit_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects").join("proj");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false);
        let acc_c = sample_account("cccc3333-0000-0000-0000-000000000003", "org-cccc", "C", false);
        let accounts = vec![acc_a.clone(), acc_b.clone(), acc_c.clone()];
        let dir_a = data_root.join("claude-code-sessions").join(&acc_a.account_uuid).join(&acc_a.org_uuid);
        for acc in &accounts {
            fs::create_dir_all(data_root.join("claude-code-sessions").join(&acc.account_uuid).join(&acc.org_uuid)).unwrap();
        }
        fs::create_dir_all(&projects_dir).unwrap();
        // f1, f2 in the picked folder (one via originCwd, other casing/trailing slash); f3 elsewhere; solo picked by id
        for (sid, cwd_field, cwd) in [
            ("f1", "cwd", r"C:\Work\App"),
            ("f2", "originCwd", "c:/work/app/"),
            ("f3", "cwd", r"C:\Work\Other"),
            ("solo", "cwd", r"C:\Work\Other"),
        ] {
            fs::write(projects_dir.join(format!("cli-{sid}.jsonl")), "{}").unwrap();
            let mut v = json!({"sessionId": format!("local_{sid}"), "cliSessionId": format!("cli-{sid}"), "title": sid, "createdAt": 1, "lastActivityAt": 2});
            v[cwd_field] = json!(cwd);
            fs::write(dir_a.join(format!("local_{sid}.json")), v.to_string()).unwrap();
        }
        let copied = |sel: &SyncSelection| {
            let preview = preview_sync(&data_root, projects_dir.parent().unwrap(), &accounts, "sandbox", Some(sel));
            let mut v: Vec<String> = preview
                .actions
                .iter()
                .filter(|a| a.action == "copy-new")
                .map(|a| format!("{}>{}", a.title, a.target_account_label))
                .collect();
            v.sort();
            v
        };
        let mut sel = SyncSelection {
            chats: ["cli-solo".to_string()].into(),
            groups: HashSet::new(),
            folders: [folder_key(r"C:\Work\App\")].into(),
            targets: HashMap::new(),
        };
        assert_eq!(copied(&sel), ["f1>B", "f1>C", "f2>B", "f2>C", "solo>B", "solo>C"]);

        // Only account C should have the folder's chats; the solo chat still goes everywhere.
        sel.targets.insert(format!("folder:{}", folder_key(r"C:\Work\App")), vec![acc_c.account_uuid.to_uppercase()]);
        assert_eq!(copied(&sel), ["f1>C", "f2>C", "solo>B", "solo>C"]);

        // A chat picked twice gets the union of both picks' targets.
        sel.chats.insert("cli-f1".to_string());
        sel.targets.insert("chat:cli-f1".to_string(), vec![acc_b.account_uuid.clone()]);
        assert_eq!(copied(&sel), ["f1>B", "f1>C", "f2>C", "solo>B", "solo>C"]);
    }

    #[test]
    fn test_a_to_b_to_a_switch_and_bidirectional_sync_without_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects");
        let backups_dir = temp.path().join("backups");

        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "Account-A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "Account-B", false);
        let accounts = vec![acc_a.clone(), acc_b.clone()];

        // 1. Setup Account A with initial session `local_s1.json` and a CLI transcript `.jsonl`
        let proj_folder = projects_dir.join("C--Users-test-myproject");
        fs::create_dir_all(&proj_folder).unwrap();
        let cli_s1 = "cli-uuid-1111";
        fs::write(
            proj_folder.join(format!("{cli_s1}.jsonl")),
            r#"{"type":"user","message":"Hello from Account A"}"#,
        )
        .unwrap();

        let dir_a = data_root
            .join("claude-code-sessions")
            .join(&acc_a.account_uuid)
            .join(&acc_a.org_uuid);
        let dir_b = data_root
            .join("claude-code-sessions")
            .join(&acc_b.account_uuid)
            .join(&acc_b.org_uuid);
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let s1_path_a = dir_a.join("local_s1.json");
        let s1_json = json!({
            "sessionId": "local_s1",
            "cliSessionId": cli_s1,
            "cwd": "C:\\Users\\test\\myproject",
            "originCwd": "C:\\Users\\test\\myproject",
            "title": "Architecture Design Session",
            "createdAt": 1762700000000i64,
            "lastActivityAt": 1762700500000i64,
            "completedTurns": 4,
            "model": "claude-opus-4-6",
            "isArchived": false,
            "isStarred": true,
            "bridgeSessionIds": ["cse_secret_account_a_token"]
        });
        fs::write(&s1_path_a, serde_json::to_vec_pretty(&s1_json).unwrap()).unwrap();
        let s1_hash_in_a_before = sha256_file(&s1_path_a).unwrap();

        // 2. Sync A -> B and switch active account to B
        let preview_ab = preview_sync(&data_root, &projects_dir, &accounts, "sandbox", None);
        assert_eq!(preview_ab.copy_new_count, 1);

        let res_ab = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, true, "sandbox", true, true, None).unwrap();
        assert_eq!(res_ab.copied_new_files, 1);
        set_active_account(&data_root, &acc_b);

        // Verify Account B now has `local_s1.json` with `bridgeSessionIds` stripped (`[]`)
        let s1_path_b = dir_b.join("local_s1.json");
        assert!(s1_path_b.is_file());
        let s1_b_val: Value = serde_json::from_str(&fs::read_to_string(&s1_path_b).unwrap()).unwrap();
        assert_eq!(s1_b_val["cliSessionId"], cli_s1);
        assert_eq!(s1_b_val["bridgeSessionIds"], json!([]));

        // Verify Account A's original file was NOT touched
        assert_eq!(sha256_file(&s1_path_a).unwrap(), s1_hash_in_a_before);

        // 3. In Account B: create a brand new session `local_s2.json` AND continue `local_s1.json`
        let cli_s2 = "cli-uuid-2222";
        fs::write(
            proj_folder.join(format!("{cli_s2}.jsonl")),
            r#"{"type":"user","message":"Created in Account B"}"#,
        )
        .unwrap();
        let s2_path_b = dir_b.join("local_s2.json");
        let s2_json = json!({
            "sessionId": "local_s2",
            "cliSessionId": cli_s2,
            "cwd": "C:\\Users\\test\\myproject",
            "title": "New Feature Built in Account B",
            "createdAt": 1762701000000i64,
            "lastActivityAt": 1762702000000i64,
            "completedTurns": 7,
            "model": "claude-sonnet-4-6",
            "isArchived": false,
            "isStarred": false,
            "bridgeSessionIds": ["cse_secret_account_b_token"]
        });
        fs::write(&s2_path_b, serde_json::to_vec_pretty(&s2_json).unwrap()).unwrap();

        // Also simulate Account B advancing `local_s1.json` to 9 turns (Desktop also renamed it)
        let s1_b_continued = json!({
            "sessionId": "local_s1",
            "cliSessionId": cli_s1,
            "cwd": "C:\\Users\\test\\myproject",
            "title": "Architecture Design Session v2",
            "createdAt": 1762700000000i64,
            "lastActivityAt": 1762703000000i64,
            "completedTurns": 9,
            "model": "claude-opus-4-6",
            "isArchived": false,
            "isStarred": true,
            "bridgeSessionIds": []
        });
        fs::write(&s1_path_b, serde_json::to_vec_pretty(&s1_b_continued).unwrap()).unwrap();

        // An old-style duplicate pointer in A (same transcript as local_s1) must not spread to B.
        let mut dup = s1_json.clone();
        dup["sessionId"] = json!("local_dup_of_s1");
        fs::write(dir_a.join("local_dup_of_s1.json"), serde_json::to_vec_pretty(&dup).unwrap()).unwrap();

        // 4. Switch B -> A and sync back
        let preview_ba = preview_sync(&data_root, &projects_dir, &accounts, "sandbox", None);
        assert_eq!(preview_ba.copy_new_count, 1, "only local_s2 should be copied to A");
        assert_eq!(
            preview_ba.conflict_companion_count, 0,
            "local_s1 continued in B shares the same transcript, so A must not get a duplicate companion"
        );
        assert_eq!(preview_ba.metadata_refresh_count, 1, "A's local_s1 is older than B's");

        let res_ba = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, true, "sandbox", true, true, None).unwrap();
        assert_eq!(res_ba.copied_new_files, 1);
        assert_eq!(res_ba.created_conflict_companions, 0);
        assert_eq!(res_ba.refreshed_metadata, 1);
        set_active_account(&data_root, &acc_a);

        // 5. A's `local_s1.json` changed ONLY in date/turns/title (bridge token etc. kept), A has the
        //    new `local_s2.json`, and B did not receive the duplicate pointer.
        let s1_a_after: Value = serde_json::from_str(&fs::read_to_string(&s1_path_a).unwrap()).unwrap();
        let without_metadata = |v: &Value| {
            let mut v = v.clone();
            for key in METADATA_KEYS {
                v.as_object_mut().unwrap().remove(key);
            }
            v
        };
        assert_eq!(without_metadata(&s1_a_after), without_metadata(&s1_json));
        for key in METADATA_KEYS {
            assert_eq!(s1_a_after[key], s1_b_continued[key], "{key} must come from B");
        }
        assert_eq!(s1_a_after["bridgeSessionIds"], json!(["cse_secret_account_a_token"]));
        assert!(dir_a.join("local_s2.json").is_file(), "local_s2 must exist in Account A");
        assert!(!dir_b.join("local_dup_of_s1.json").exists(), "duplicate must not spread to B");

        // 6. Syncing again changes nothing: sessions never multiply and nothing is refreshed twice.
        let again = preview_sync(&data_root, &projects_dir, &accounts, "sandbox", None);
        assert_eq!(again.copy_new_count + again.conflict_companion_count + again.metadata_refresh_count, 0);

        // 7. A background sync with nothing to do must not create another (large) backup.
        let backups_before = fs::read_dir(&backups_dir).unwrap().count();
        let idle = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, false, "sandbox", true, true, None).unwrap();
        assert!(idle.pre_sync_backup_id.is_empty());
        assert_eq!(idle.refreshed_metadata, 0);
        assert_eq!(fs::read_dir(&backups_dir).unwrap().count(), backups_before);
    }

    #[test]
    fn test_metadata_refresh_waits_for_desktop_closed_and_forces_backup() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects");
        let backups_dir = temp.path().join("backups");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "Account-A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "Account-B", false);
        let accounts = vec![acc_a.clone(), acc_b.clone()];

        fs::create_dir_all(projects_dir.join("proj")).unwrap();
        fs::write(projects_dir.join("proj").join("cli-1.jsonl"), "{}").unwrap();
        let pointer = |dir: &Path, turns: i64, last: i64, title: &str| {
            fs::create_dir_all(dir).unwrap();
            let v = json!({"sessionId": "local_s1", "cliSessionId": "cli-1", "title": title,
                "createdAt": 1i64, "lastActivityAt": last, "completedTurns": turns,
                "bridgeSessionIds": ["tok"], "someUnknownKey": {"x": 1}});
            let path = dir.join("local_s1.json");
            fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
            path
        };
        let path_a = pointer(&data_root.join("claude-code-sessions").join(&acc_a.account_uuid).join("org-aaaa"), 2, 100, "old");
        pointer(&data_root.join("claude-code-sessions").join(&acc_b.account_uuid).join("org-bbbb"), 5, 200, "new");
        let hash_a = sha256_file(&path_a).unwrap();

        // Desktop running: preview still reports the refresh, but nothing is written and no backup is forced.
        assert_eq!(preview_sync(&data_root, &projects_dir, &accounts, "sandbox", None).metadata_refresh_count, 1);
        let res = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, false, "sandbox", false, false, None).unwrap();
        assert_eq!(res.refreshed_metadata, 0);
        assert!(res.pre_sync_backup_id.is_empty());
        assert_eq!(sha256_file(&path_a).unwrap(), hash_a);
        assert!(!backups_dir.exists() || fs::read_dir(&backups_dir).unwrap().count() == 0);

        // Desktop closed: A is refreshed, with a backup even though take_backup=false.
        let res = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, false, "sandbox", false, true, None).unwrap();
        assert_eq!(res.refreshed_metadata, 1);
        assert!(!res.pre_sync_backup_id.starts_with("none"));
        let a: Value = serde_json::from_str(&fs::read_to_string(&path_a).unwrap()).unwrap();
        assert_eq!((a["lastActivityAt"].as_i64(), a["completedTurns"].as_i64(), a["title"].as_str()), (Some(200), Some(5), Some("new")));
        assert_eq!(a["someUnknownKey"], json!({"x": 1}));

        // Stale-write guard: a target that changed since the preview is left alone.
        let source = json!({"cliSessionId": "cli-1", "lastActivityAt": 300});
        assert!(refresh_metadata_json(r#"{"cliSessionId":"cli-1","lastActivityAt":250}"#, &source, 200, None).is_none());
        assert!(refresh_metadata_json(r#"{"cliSessionId":"cli-2","lastActivityAt":200}"#, &source, 200, None).is_none());
        assert!(refresh_metadata_json("not json", &source, 200, None).is_none());
    }

    #[test]
    fn changed_format_blocks_every_write() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects");
        let backups_dir = temp.path().join("backups");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false);
        let accounts = vec![acc_a.clone(), acc_b.clone()];
        let dir_a = data_root.join("claude-code-sessions").join(&acc_a.account_uuid).join(&acc_a.org_uuid);
        let dir_b = data_root.join("claude-code-sessions").join(&acc_b.account_uuid).join(&acc_b.org_uuid);
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();
        fs::create_dir_all(projects_dir.join("proj")).unwrap();
        // Dates became ISO strings: scan falls back to defaults, so without the guard these would be copied.
        for i in 0..3 {
            fs::write(projects_dir.join("proj").join(format!("cli-{i}.jsonl")), "{}").unwrap();
            let v = json!({"sessionId": format!("local_{i}"), "cliSessionId": format!("cli-{i}"), "title": "t",
                "createdAt": "2026-11-10T10:00:00Z", "lastActivityAt": "2026-11-10T11:00:00Z"});
            fs::write(dir_a.join(format!("local_{i}.json")), v.to_string()).unwrap();
        }
        assert_eq!(preview_sync(&data_root, &projects_dir, &accounts, "sandbox", None).copy_new_count, 3);

        for _ in 0..2 {
            let err = execute_sync_inner(&data_root, &projects_dir, &backups_dir, &accounts, true, "sandbox", true, true, None).unwrap_err();
            assert!(crate::format_guard::is_guard_error(&err), "{err}");
        }
        assert_eq!(fs::read_dir(&dir_b).unwrap().count(), 0, "nothing may be written");
        let names: Vec<String> = fs::read_dir(&backups_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["journal.jsonl"], "no backup either");
        let journal = crate::backup::read_journal(&backups_dir, 10);
        assert_eq!(journal.len(), 1, "one journal entry, not one per attempt");
        assert_eq!(journal[0].operation, "format-guard");
    }

    /// Pointer `local_<sid>` in `acc`'s folder; `healthy` = transcript on disk, else `transcriptUnavailable`.
    fn write_chat(data_root: &Path, projects_dir: &Path, acc: &AccountProfile, sid: &str, last: i64, healthy: bool) {
        let dir = data_root.join("claude-code-sessions").join(&acc.account_uuid).join(&acc.org_uuid);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(projects_dir).unwrap();
        if healthy {
            fs::write(projects_dir.join(format!("cli-{sid}.jsonl")), "{}").unwrap();
        }
        let v = json!({"sessionId": format!("local_{sid}"), "cliSessionId": format!("cli-{sid}"), "title": sid, "createdAt": 1, "lastActivityAt": last, "completedTurns": 1, "transcriptUnavailable": !healthy});
        fs::write(dir.join(format!("local_{sid}.json")), v.to_string()).unwrap();
    }

    fn empty_selection() -> SyncSelection {
        SyncSelection { chats: HashSet::new(), groups: HashSet::new(), folders: HashSet::new(), targets: HashMap::new() }
    }

    #[test]
    fn ask_mode_refreshes_existing_copies_but_copies_no_unapproved_chat() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects").join("proj");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false);
        let accounts = vec![acc_a.clone(), acc_b.clone()];
        write_chat(&data_root, &projects_dir, &acc_a, "shared", 5, true);
        write_chat(&data_root, &projects_dir, &acc_b, "shared", 2, true);
        write_chat(&data_root, &projects_dir, &acc_a, "fresh", 3, true);

        let sel = empty_selection();
        let p = preview_sync(&data_root, projects_dir.parent().unwrap(), &accounts, "sandbox", Some(&sel));
        let acts: Vec<String> = p.actions.iter().map(|a| format!("{}:{}>{}", a.action, a.title, a.target_account_label)).collect();
        assert!(acts.contains(&"metadata-refresh:shared>B".to_string()), "{acts:?}");
        assert_eq!(p.copy_new_count, 0, "{acts:?}");
        // "all" mode copies the new chat too
        assert_eq!(preview_sync(&data_root, projects_dir.parent().unwrap(), &accounts, "sandbox", None).copy_new_count, 1);
    }

    #[test]
    fn switch_plan_lists_only_missing_unapproved_healthy_chats() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects").join("proj");
        let acc_a = sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true);
        let acc_b = sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false);
        let acc_c = sample_account("cccc3333-0000-0000-0000-000000000003", "org-cccc", "C", false);
        let accounts = vec![acc_a.clone(), acc_b.clone(), acc_c.clone()];
        write_chat(&data_root, &projects_dir, &acc_a, "shared", 5, true); // B already lists it
        write_chat(&data_root, &projects_dir, &acc_b, "shared", 2, true);
        write_chat(&data_root, &projects_dir, &acc_a, "picked", 6, true); // approved for all
        write_chat(&data_root, &projects_dir, &acc_a, "only-c", 7, true); // approved for C only
        write_chat(&data_root, &projects_dir, &acc_a, "old", 3, true);
        write_chat(&data_root, &projects_dir, &acc_a, "new", 9, true);
        write_chat(&data_root, &projects_dir, &acc_a, "broken", 8, false); // no transcript
        let projects = projects_dir.parent().unwrap();

        let mut sel = empty_selection();
        sel.chats.extend(["cli-picked".to_string(), "cli-only-c".to_string()]);
        sel.targets.insert("chat:cli-only-c".to_string(), vec![acc_c.account_uuid.clone()]);
        let pending = pending_for_target(&data_root, projects, &accounts, Some(&sel), &acc_b.account_uuid.to_uppercase());
        let ids: Vec<&str> = pending.iter().map(|p| p.cli_session_id.as_str()).collect();
        assert_eq!(ids, ["cli-new", "cli-only-c", "cli-old"]);
        assert_eq!(pending[0].session_id, "local_new");
        assert_eq!(pending[0].source_account_uuid, acc_a.account_uuid);

        assert!(pending_for_target(&data_root, projects, &accounts, None, &acc_b.account_uuid).is_empty());
    }

    /// Two accounts with empty session folders and a `proj` transcript folder.
    struct Env {
        _temp: tempfile::TempDir,
        data_root: PathBuf,
        projects_dir: PathBuf,
        backups_dir: PathBuf,
        accounts: Vec<AccountProfile>,
    }

    fn env() -> Env {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("claude-data");
        let projects_dir = temp.path().join("cli-projects");
        let backups_dir = temp.path().join("backups");
        let accounts = vec![
            sample_account("aaaa1111-0000-0000-0000-000000000001", "org-aaaa", "A", true),
            sample_account("bbbb2222-0000-0000-0000-000000000002", "org-bbbb", "B", false),
        ];
        for acc in &accounts {
            fs::create_dir_all(data_root.join("claude-code-sessions").join(&acc.account_uuid).join(&acc.org_uuid)).unwrap();
        }
        fs::create_dir_all(projects_dir.join("proj")).unwrap();
        Env { _temp: temp, data_root, projects_dir, backups_dir, accounts }
    }

    impl Env {
        fn dir(&self, i: usize) -> PathBuf {
            let acc = &self.accounts[i];
            self.data_root.join("claude-code-sessions").join(&acc.account_uuid).join(&acc.org_uuid)
        }
        /// Writes pointer `sid` (with its transcript) into account `i`; returns its path and JSON.
        fn pointer(&self, i: usize, sid: &str, cli: &str, prior: &[&str], last: i64, turns: i64, title: &str) -> (PathBuf, Value) {
            fs::write(self.projects_dir.join("proj").join(format!("{cli}.jsonl")), "{}").unwrap();
            let v = json!({"sessionId": sid, "cliSessionId": cli, "priorCliSessionIds": prior, "title": title,
                "createdAt": 1i64, "lastActivityAt": last, "completedTurns": turns, "model": format!("model-{i}"),
                "bridgeSessionIds": [format!("tok-{i}")], "someUnknownKey": {"acc": i}});
            let path = self.dir(i).join(format!("{sid}.json"));
            fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
            (path, v)
        }
        fn preview(&self) -> SyncPreviewReport {
            preview_sync(&self.data_root, &self.projects_dir, &self.accounts, "sandbox", None)
        }
        fn run(&self, desktop_closed: bool) -> SyncExecutionResult {
            execute_sync_inner(&self.data_root, &self.projects_dir, &self.backups_dir, &self.accounts, false, "sandbox", false, desktop_closed, None).unwrap()
        }
        fn backups(&self) -> usize {
            fs::read_dir(&self.backups_dir).map(|d| d.flatten().filter(|e| e.path().is_dir()).count()).unwrap_or(0)
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn writes(p: &SyncPreviewReport) -> usize {
        p.copy_new_count + p.conflict_companion_count + p.metadata_refresh_count + p.continue_refresh_count
    }

    #[test]
    fn continued_chat_updates_the_existing_pointer_in_place() {
        let e = env();
        let (path_b, b_before) = e.pointer(1, "local_s1", "cli-old", &["cli-older"], 100, 3, "old title");
        let (path_a, a_json) = e.pointer(0, "local_s1", "cli-new", &["cli-older", "cli-old"], 200, 8, "new title");
        let hash_a = sha256_file(&path_a).unwrap();
        let hash_b = sha256_file(&path_b).unwrap();

        let p = e.preview();
        assert_eq!((p.continue_refresh_count, p.conflict_companion_count, p.copy_new_count), (1, 0, 0), "{:?}", p.actions);
        // "ask" mode: no approval needed, and the continuation is not offered as a new chat either.
        let ask = preview_sync(&e.data_root, &e.projects_dir, &e.accounts, "sandbox", Some(&empty_selection()));
        assert_eq!(ask.continue_refresh_count, 1);
        assert!(pending_for_target(&e.data_root, &e.projects_dir, &e.accounts, Some(&empty_selection()), &e.accounts[1].account_uuid).is_empty());

        // Desktop running: nothing written, no backup.
        let res = e.run(false);
        assert_eq!(res.continued_pointers, 0);
        assert!(res.pre_sync_backup_id.is_empty());
        assert_eq!(sha256_file(&path_b).unwrap(), hash_b);
        assert_eq!(e.backups(), 0);

        // Desktop closed: updated in place after a forced backup (take_backup = false).
        let res = e.run(true);
        assert_eq!((res.continued_pointers, res.created_conflict_companions, res.refreshed_metadata), (1, 0, 0));
        assert!(!res.pre_sync_backup_id.is_empty() && !res.pre_sync_backup_id.starts_with("none"));
        assert_eq!(e.backups(), 1);
        let b_after = read_json(&path_b);
        let strip = |v: &Value| {
            let mut v = v.clone();
            for key in CONTINUE_KEYS {
                v.as_object_mut().unwrap().remove(key);
            }
            v
        };
        assert_eq!(strip(&b_after), strip(&b_before), "only the five keys may change");
        for key in CONTINUE_KEYS {
            assert_eq!(b_after[key], a_json[key], "{key} must come from A");
        }
        assert_eq!(b_after["bridgeSessionIds"], json!(["tok-1"]));
        assert_eq!(sha256_file(&path_a).unwrap(), hash_a, "source untouched");
        assert_eq!(fs::read_dir(e.dir(1)).unwrap().count(), 1, "no companion");
        let journal = crate::backup::read_journal(&e.backups_dir, 1);
        assert!(journal[0].details.contains("1 continued pointers updated"), "{}", journal[0].details);

        // No ping-pong: the pair is identical now, and an idle run takes no backup.
        assert_eq!(writes(&e.preview()), 0);
        assert!(e.run(true).pre_sync_backup_id.is_empty());
    }

    #[test]
    fn continue_refresh_stale_guard() {
        let e = env();
        let (path_b, _) = e.pointer(1, "local_s1", "cli-old", &[], 100, 3, "old");
        let (path_a, _) = e.pointer(0, "local_s1", "cli-new", &["cli-old"], 200, 8, "new");
        let p = e.preview();
        let item = p.actions.iter().find(|a| a.action == "continue-refresh").unwrap();
        let source = read_json(&path_a);
        assert!(refreshed_bytes(item, &fs::read_to_string(&path_b).unwrap(), &source).is_some());

        // Target changed since the preview (date or transcript id): left alone.
        assert!(refreshed_bytes(item, r#"{"cliSessionId":"cli-old","lastActivityAt":150}"#, &source).is_none());
        assert!(refreshed_bytes(item, r#"{"cliSessionId":"cli-other","lastActivityAt":100}"#, &source).is_none());
        // Source no longer lists the target's id as prior, or is not newer.
        let not_prior = json!({"cliSessionId": "cli-new", "priorCliSessionIds": "cli-old", "lastActivityAt": 200});
        assert!(refreshed_bytes(item, r#"{"cliSessionId":"cli-old","lastActivityAt":100}"#, &not_prior).is_none());
        let older = json!({"cliSessionId": "cli-new", "priorCliSessionIds": ["cli-old"], "lastActivityAt": 90});
        assert!(refreshed_bytes(item, r#"{"cliSessionId":"cli-old","lastActivityAt":100}"#, &older).is_none());

        // Desktop rewrote the target between preview and write: execute skips it.
        let mut changed = read_json(&path_b);
        changed["lastActivityAt"] = json!(150);
        fs::write(&path_b, serde_json::to_vec_pretty(&changed).unwrap()).unwrap();
        let hash_b = sha256_file(&path_b).unwrap();
        let res = execute_sync_inner(&e.data_root, &e.projects_dir, &e.backups_dir, &e.accounts, false, "sandbox", false, true, None).unwrap();
        assert_eq!(res.created_conflict_companions, 0);
        // The fresh preview inside execute sees 150 and updates it; that is the newest state, not a stale write.
        assert_eq!(res.continued_pointers, 1);
        assert_ne!(sha256_file(&path_b).unwrap(), hash_b);
        assert_eq!(read_json(&path_b)["cliSessionId"], "cli-new");
    }

    #[test]
    fn truly_diverged_chat_still_gets_a_companion() {
        let e = env();
        e.pointer(1, "local_s1", "cli-b", &[], 100, 3, "t");
        e.pointer(0, "local_s1", "cli-a", &["cli-x"], 200, 8, "t");
        let p = e.preview();
        assert_eq!((p.conflict_companion_count, p.continue_refresh_count), (1, 0));
        let res = e.run(true);
        assert_eq!((res.created_conflict_companions, res.continued_pointers), (1, 0));
        assert_eq!(read_json(&e.dir(1).join("local_s1.json"))["cliSessionId"], "cli-b", "original untouched");
    }

    #[test]
    fn target_ahead_is_never_overwritten_with_older_data() {
        // B continued the chat (cli-new, prior cli-old) and is newer: B is the source, A gets moved along.
        let e = env();
        let (path_a, _) = e.pointer(0, "local_s1", "cli-old", &[], 100, 3, "old");
        let (path_b, _) = e.pointer(1, "local_s1", "cli-new", &["cli-old"], 200, 8, "new");
        let hash_b = sha256_file(&path_b).unwrap();
        let p = e.preview();
        let acts: Vec<String> = p.actions.iter().map(|a| format!("{}>{}", a.action, a.target_account_label)).collect();
        assert_eq!(acts, ["continue-refresh>A"]);
        e.run(true);
        assert_eq!(read_json(&path_a)["cliSessionId"], "cli-new");
        assert_eq!(sha256_file(&path_b).unwrap(), hash_b);
        assert_eq!(writes(&e.preview()), 0);

        // The best-source rule picks the OLD side (more turns): the newer target is skipped, not overwritten.
        let e = env();
        e.pointer(0, "local_s1", "cli-old", &[], 100, 50, "old");
        let (path_b, _) = e.pointer(1, "local_s1", "cli-new", &["cli-old"], 200, 8, "new");
        let hash_b = sha256_file(&path_b).unwrap();
        let p = e.preview();
        assert_eq!(writes(&p), 0, "{:?}", p.actions);
        let res = e.run(true);
        assert_eq!((res.continued_pointers, res.created_conflict_companions, res.copied_new_files), (0, 0, 0));
        assert_eq!(sha256_file(&path_b).unwrap(), hash_b);
        assert_eq!(fs::read_dir(e.dir(1)).unwrap().count(), 1);
    }

    #[test]
    fn old_companion_stays_and_original_pointer_catches_up() {
        let e = env();
        let (orig_b, _) = e.pointer(1, "local_s1", "cli-1", &[], 100, 3, "chat");
        let (comp_b, _) = e.pointer(1, "local_comp", "cli-2", &["cli-1"], 150, 5, "chat [Continued in A]");
        let comp_hash = sha256_file(&comp_b).unwrap();
        e.pointer(0, "local_s1", "cli-2", &["cli-1"], 200, 6, "chat");

        // The companion already lists cli-2: the original is still moved along, nothing new is created.
        let p = e.preview();
        assert_eq!((p.continue_refresh_count, p.conflict_companion_count, p.copy_new_count), (1, 0, 0), "{:?}", p.actions);
        e.run(true);
        assert_eq!(read_json(&orig_b)["cliSessionId"], "cli-2");

        // The chat continues again in A: B's original follows, the companion is not copied back to A.
        e.pointer(0, "local_s1", "cli-3", &["cli-1", "cli-2"], 300, 9, "chat");
        let p = e.preview();
        assert_eq!((p.continue_refresh_count, p.conflict_companion_count, p.copy_new_count), (1, 0, 0), "{:?}", p.actions);
        e.run(true);
        let orig = read_json(&orig_b);
        assert_eq!((orig["cliSessionId"].as_str(), orig["priorCliSessionIds"].clone()), (Some("cli-3"), json!(["cli-1", "cli-2"])));
        assert_eq!(sha256_file(&comp_b).unwrap(), comp_hash, "old companion untouched");
        assert_eq!(fs::read_dir(e.dir(1)).unwrap().count(), 2);
        assert_eq!(fs::read_dir(e.dir(0)).unwrap().count(), 1);
        assert_eq!(writes(&e.preview()), 0);
    }

    #[test]
    fn prior_ids_tolerate_missing_or_odd_values() {
        assert!(prior_cli_ids(&json!({})).is_empty());
        assert!(prior_cli_ids(&json!({"priorCliSessionIds": "x"})).is_empty());
        assert_eq!(prior_cli_ids(&json!({"priorCliSessionIds": ["a", 1, null, "", "b"]})), ["a", "b"]);
    }
}
