//! Backup, SHA-256 integrity verification, and non-destructive restore engine.
//!
//! Rules enforced:
//! 1. All backups are stored in an external backup directory (`%LOCALAPPDATA%\ClaudeSessionSyncBackups\`),
//!    never inside the source session folders.
//! 2. Every backed-up file is hashed with SHA-256 before and after copy; if any hash mismatches,
//!    the backup fails immediately and reports the error.
//! 3. Restore never overwrites existing files: it only adds missing ones, after a pre-restore backup.
//! 4. Includes a persistent append-only transaction journal (`journal.jsonl`) for auditability.

use crate::dpapi::{atomic_write_verified, is_file_locked, sha256_file};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFileEntry {
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub id: String,
    pub created_at: String,
    pub reason: String,
    pub source_root: String,
    pub backup_dir: String,
    pub total_files: usize,
    pub total_bytes: u64,
    pub all_verified: bool,
    pub files: Vec<BackupFileEntry>,
    /// `None` = full snapshot; `Some(LIVE_REFRESH_BACKUP)` = only the pointers a refresh changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `BackupManifest::kind` of the small backups taken before refreshing pointers while Desktop runs.
pub const LIVE_REFRESH_BACKUP: &str = "live-refresh";
/// How many live-refresh backups pruning keeps (they come every 15 minutes all day).
const KEEP_LIVE_REFRESH: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub backup_id: String,
    pub pre_restore_backup_id: String,
    pub created_files: usize,
    pub preserved_conflicts: usize,
    pub skipped_identical: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub timestamp: String,
    pub operation: String,
    pub mode: String,
    pub details: String,
    pub status: String,
}

/// Append a record to the transaction journal (`journal.jsonl`).
pub fn append_journal(backup_storage_dir: &Path, op: &str, mode: &str, details: &str, status: &str) {
    let _ = fs::create_dir_all(backup_storage_dir);
    let journal_path = backup_storage_dir.join("journal.jsonl");
    let entry = JournalEntry {
        timestamp: Utc::now().to_rfc3339(),
        operation: op.to_string(),
        mode: mode.to_string(),
        details: details.to_string(),
        status: status.to_string(),
    };
    if let Ok(line) = serde_json::to_string(&entry) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(journal_path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Read recent journal entries (newest first).
pub fn read_journal(backup_storage_dir: &Path, limit: usize) -> Vec<JournalEntry> {
    let journal_path = backup_storage_dir.join("journal.jsonl");
    let Ok(text) = fs::read_to_string(journal_path) else {
        return Vec::new();
    };
    let mut entries: Vec<JournalEntry> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<JournalEntry>(l).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

fn collect_files_recursive(base: &Path, current: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !current.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(base, &path, out)?;
        } else if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains(".cs-tmp-") || name.eq_ignore_ascii_case("LOCK") {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}

/// Create a full, SHA-256 verified backup of the session data in `source_data_root`
/// into an isolated subfolder under `backup_storage_dir`.
pub fn create_verified_backup(
    source_data_root: &Path,
    backup_storage_dir: &Path,
    reason: &str,
    mode: &str,
) -> Result<BackupManifest, String> {
    let targets = [
        "claude-code-sessions",
        "local-agent-mode-sessions",
        "claude_desktop_config.json",
        "config.json",
    ];

    let mut files_to_copy: Vec<PathBuf> = Vec::new();
    for rel in &targets {
        let candidate = source_data_root.join(rel);
        if candidate.is_dir() {
            collect_files_recursive(source_data_root, &candidate, &mut files_to_copy)
                .map_err(|e| format!("Failed scanning {}: {e}", candidate.display()))?;
        } else if candidate.is_file() {
            files_to_copy.push(candidate);
        }
    }

    files_to_copy.sort();
    create_verified_file_backup(source_data_root, backup_storage_dir, &files_to_copy, reason, mode, None)
}

/// SHA-256 verified backup of just `files` (all under `source_data_root`), laid out and listed
/// like a full backup so restore, `list_backups`, pruning and the health check treat it the same.
pub fn create_verified_file_backup(
    source_data_root: &Path,
    backup_storage_dir: &Path,
    files: &[PathBuf],
    reason: &str,
    mode: &str,
    kind: Option<&str>,
) -> Result<BackupManifest, String> {
    fs::create_dir_all(backup_storage_dir)
        .map_err(|e| format!("Failed to create backup storage root: {e}"))?;

    let now = Utc::now();
    let id = format!(
        "backup-{}-{}",
        now.format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let dest_root = backup_storage_dir.join(&id);
    fs::create_dir_all(&dest_root)
        .map_err(|e| format!("Failed to create backup folder {}: {e}", dest_root.display()))?;

    let mut manifest_entries = Vec::with_capacity(files.len());
    let mut total_bytes = 0u64;

    for src_file in files {
        let rel_path = src_file
            .strip_prefix(source_data_root)
            .map_err(|e| format!("Strip prefix error: {e}"))?
            .to_string_lossy()
            .replace('\\', "/");

        let dst_file = dest_root.join(&rel_path);
        if let Some(parent) = dst_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed creating backup subdir {}: {e}", parent.display()))?;
        }

        let src_hash = sha256_file(src_file)
            .map_err(|e| format!("Failed reading source file {} for SHA-256: {e}", src_file.display()))?;
        let meta = fs::metadata(src_file)
            .map_err(|e| format!("Failed reading metadata {}: {e}", src_file.display()))?;
        let size_bytes = meta.len();

        fs::copy(src_file, &dst_file)
            .map_err(|e| format!("Failed copying {} to backup: {e}", src_file.display()))?;

        let dst_hash = sha256_file(&dst_file)
            .map_err(|e| format!("Failed hashing backed up file {}: {e}", dst_file.display()))?;

        if src_hash != dst_hash {
            let _ = fs::remove_dir_all(&dest_root);
            append_journal(
                backup_storage_dir,
                "backup",
                mode,
                &format!("SHA-256 mismatch on {rel_path}"),
                "error",
            );
            return Err(format!(
                "CRITICAL: Backup SHA-256 verification failed for {rel_path} (source {src_hash} != backup {dst_hash})"
            ));
        }

        total_bytes += size_bytes;
        manifest_entries.push(BackupFileEntry {
            relative_path: rel_path,
            sha256: dst_hash,
            size_bytes,
            verified: true,
        });
    }

    let manifest = BackupManifest {
        id: id.clone(),
        created_at: now.to_rfc3339(),
        reason: reason.to_string(),
        source_root: source_data_root.to_string_lossy().into_owned(),
        backup_dir: dest_root.to_string_lossy().into_owned(),
        total_files: manifest_entries.len(),
        total_bytes,
        all_verified: true,
        files: manifest_entries,
        kind: kind.map(String::from),
    };

    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("Failed serializing backup manifest: {e}"))?;
    atomic_write_verified(&dest_root.join("backup-manifest.json"), &manifest_json)?;

    append_journal(
        backup_storage_dir,
        "backup",
        mode,
        &format!(
            "Created verified backup {} ({} files, {} bytes) - reason: {}",
            id, manifest.total_files, manifest.total_bytes, reason
        ),
        "ok",
    );

    prune_old_backups(backup_storage_dir, &manifest.id, kind == Some(LIVE_REFRESH_BACKUP));

    Ok(manifest)
}

/// Delete old automatic backups. Keeps: the last 7 days, the newest 10 automatic ones,
/// `keep_id`, manual backups and anything that is not a `backup-*` dir with a parsable manifest.
/// Live-refresh backups have their own rule: the newest 20 (counting `keep_id` when it is one,
/// `keep_is_live`), whatever their age; they never take a slot of the newest 10.
/// Never fails the caller; deletions are journaled.
fn prune_old_backups(backup_storage_dir: &Path, keep_id: &str, keep_is_live: bool) {
    let Ok(entries) = fs::read_dir(backup_storage_dir) else {
        return;
    };
    let cutoff = Utc::now() - chrono::Duration::days(7);
    // (created_at, path, id) of automatic backups only
    let mut auto: Vec<(chrono::DateTime<chrono::FixedOffset>, PathBuf, String)> = Vec::new();
    let mut live: Vec<(chrono::DateTime<chrono::FixedOffset>, PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !path.is_dir() || !name.starts_with("backup-") || name == keep_id {
            continue;
        }
        let Ok(text) = fs::read_to_string(path.join("backup-manifest.json")) else {
            continue;
        };
        let Ok(m) = serde_json::from_str::<BackupManifest>(&text) else {
            continue;
        };
        if m.reason.to_lowercase().contains("manuel") {
            continue;
        }
        // unparsable date: keep, we cannot tell how old it is
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&m.created_at) {
            if m.kind.as_deref() == Some(LIVE_REFRESH_BACKUP) {
                live.push((t, path, name));
            } else {
                auto.push((t, path, name));
            }
        }
    }
    live.sort_by(|a, b| b.0.cmp(&a.0));
    let live_old = live.into_iter().skip(KEEP_LIVE_REFRESH - usize::from(keep_is_live));
    auto.sort_by(|a, b| b.0.cmp(&a.0));
    // a new full backup is itself one of the newest 10, so it takes a slot: keep 9 others
    let auto_old = auto.into_iter().skip(if keep_is_live { 10 } else { 9 }).filter(|(t, _, _)| t.with_timezone(&Utc) < cutoff);
    for (_, path, name) in live_old.chain(auto_old) {
        match fs::remove_dir_all(&path) {
            Ok(()) => append_journal(backup_storage_dir, "prune", "auto", &format!("Deleted old backup {name}"), "ok"),
            Err(e) => append_journal(backup_storage_dir, "prune", "auto", &format!("Failed to delete {name}: {e}"), "error"),
        }
    }
}

/// List all available verified backups in `backup_storage_dir` (newest first).
pub fn list_backups(backup_storage_dir: &Path) -> Vec<BackupManifest> {
    let Ok(entries) = fs::read_dir(backup_storage_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("backup-manifest.json");
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            if let Ok(m) = serde_json::from_str::<BackupManifest>(&text) {
                out.push(m);
                continue;
            }
        }
        // Also recognize initial PowerShell read-only backup with sha256-manifest.json
        let ps_manifest = path.join("sha256-manifest.json");
        if ps_manifest.is_file() {
            let folder_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if let Ok(text) = fs::read_to_string(&ps_manifest) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(arr) = val.as_array() {
                        let mut files = Vec::new();
                        let mut total_bytes = 0u64;
                        for item in arr {
                            let rel = item
                                .get("RelativePath")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .replace('\\', "/");
                            let hash = item
                                .get("BackupHash")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let sz = item.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                            total_bytes += sz;
                            files.push(BackupFileEntry {
                                relative_path: rel,
                                sha256: hash,
                                size_bytes: sz,
                                verified: true,
                            });
                        }
                        out.push(BackupManifest {
                            id: folder_name,
                            created_at: "2026-11-09T23:31:43+03:00".to_string(),
                            reason: "Initial Read-Only Safety Snapshot (Pre-Development)".to_string(),
                            source_root: "Real Claude-Data Root".to_string(),
                            backup_dir: path.to_string_lossy().into_owned(),
                            total_files: files.len(),
                            total_bytes,
                            all_verified: true,
                            files,
                            kind: None,
                        });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Verify every file in a backup against its recorded SHA-256 hash.
pub fn verify_backup_integrity(backup_storage_dir: &Path, backup_id: &str) -> Result<bool, String> {
    let backups = list_backups(backup_storage_dir);
    let manifest = backups
        .into_iter()
        .find(|b| b.id == backup_id)
        .ok_or_else(|| format!("Backup '{backup_id}' not found"))?;
    let base = PathBuf::from(&manifest.backup_dir);
    for f in &manifest.files {
        let p = base.join(&f.relative_path);
        if !p.is_file() {
            return Err(format!("Missing file in backup: {}", f.relative_path));
        }
        let actual = sha256_file(&p)
            .map_err(|e| format!("Failed hashing {}: {e}", p.display()))?;
        if !actual.eq_ignore_ascii_case(&f.sha256) {
            return Err(format!(
                "SHA-256 mismatch in backup {}: expected {}, got {}",
                f.relative_path, f.sha256, actual
            ));
        }
    }
    Ok(true)
}

/// One `claude-code-sessions/` file of a backup, as `preview_restore` sees it against the live root.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewFile {
    pub relative_path: String,
    /// `missing` (restore would create it) | `different` (kept as it is) | `identical`.
    pub status: String,
    pub title: String,
    pub last_activity_at: Option<i64>,
    pub account_uuid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub backup_id: String,
    pub files: Vec<RestorePreviewFile>,
}

const SESSIONS_PREFIX: &str = "claude-code-sessions/";

fn find_manifest(backup_storage_dir: &Path, backup_id: &str) -> Result<BackupManifest, String> {
    list_backups(backup_storage_dir)
        .into_iter()
        .find(|b| b.id == backup_id)
        .ok_or_else(|| format!("Backup '{backup_id}' not found"))
}

/// `missing` | `different` | `identical` for the live copy of a backup entry.
fn live_status(dst: &Path, sha256: &str) -> Result<&'static str, String> {
    if !dst.is_file() {
        return Ok("missing");
    }
    let cur = sha256_file(dst).map_err(|e| format!("Failed hashing {}: {e}", dst.display()))?;
    Ok(if cur.eq_ignore_ascii_case(sha256) { "identical" } else { "different" })
}

/// Read-only: verifies the backup and says, per session file, what a restore would do. Writes nothing.
pub fn preview_restore(backup_storage_dir: &Path, backup_id: &str, target_root: &Path) -> Result<RestorePreview, String> {
    verify_backup_integrity(backup_storage_dir, backup_id)?;
    let manifest = find_manifest(backup_storage_dir, backup_id)?;
    let base = PathBuf::from(&manifest.backup_dir);
    let mut files = Vec::new();
    for f in manifest.files.iter().filter(|f| f.relative_path.starts_with(SESSIONS_PREFIX)) {
        let status = live_status(&target_root.join(&f.relative_path), &f.sha256)?;
        // Not every file is a pointer, and real pointers may lack a title: fall back to the file name.
        let v: serde_json::Value = fs::read(base.join(&f.relative_path))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let file_name = f.relative_path.rsplit('/').next().unwrap_or(&f.relative_path);
        files.push(RestorePreviewFile {
            relative_path: f.relative_path.clone(),
            status: status.to_string(),
            title: v.get("title").and_then(|t| t.as_str()).filter(|t| !t.trim().is_empty()).unwrap_or(file_name).to_string(),
            last_activity_at: v.get("lastActivityAt").and_then(|n| n.as_i64()),
            account_uuid: f.relative_path[SESSIONS_PREFIX.len()..].split('/').next().unwrap_or("").to_string(),
        });
    }
    Ok(RestorePreview { backup_id: backup_id.to_string(), files })
}

/// Execute a non-destructive restore after user confirmation.
/// Never overwrites existing files. Takes a pre-restore backup first (only when something
/// would be created), verifies SHA-256 of every backup file before restoring, and if a file
/// already exists with different content, keeps the existing file intact.
/// `only`: restore just these manifest paths (`None` = every session file of the backup).
pub fn execute_restore(
    backup_storage_dir: &Path,
    backup_id: &str,
    target_root: &Path,
    mode: &str,
    only: Option<&[String]>,
) -> Result<RestoreResult, String> {
    verify_backup_integrity(backup_storage_dir, backup_id)?;
    let manifest = find_manifest(backup_storage_dir, backup_id)?;
    let backup_base = PathBuf::from(&manifest.backup_dir);
    let sessions: Vec<&BackupFileEntry> =
        manifest.files.iter().filter(|f| f.relative_path.starts_with(SESSIONS_PREFIX)).collect();

    // Requested paths are only looked up in the manifest, never joined onto a directory.
    let chosen: Vec<&BackupFileEntry> = match only {
        None => sessions,
        Some(paths) => {
            let known: std::collections::HashSet<&str> = sessions.iter().map(|f| f.relative_path.as_str()).collect();
            if let Some(bad) = paths.iter().find(|p| !known.contains(p.as_str())) {
                return Err(format!("Restore aborted: '{bad}' is not a chat file of backup {backup_id}."));
            }
            let wanted: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
            sessions.into_iter().filter(|f| wanted.contains(f.relative_path.as_str())).collect()
        }
    };

    let mut to_create = Vec::new();
    let mut preserved_conflicts = 0;
    let mut skipped_identical = 0;
    for f in chosen {
        let dst = target_root.join(&f.relative_path);
        if is_file_locked(&dst) {
            return Err(format!(
                "Restore aborted: Target file {} is locked by another process.",
                dst.display()
            ));
        }
        match live_status(&dst, &f.sha256)? {
            "identical" => skipped_identical += 1,
            // Never overwrite existing session file!
            "different" => preserved_conflicts += 1,
            _ => to_create.push((f, dst)),
        }
    }

    // Nothing to add: a full pre-restore snapshot would protect nothing.
    let pre_restore_backup_id = if to_create.is_empty() {
        String::new()
    } else {
        create_verified_backup(
            target_root,
            backup_storage_dir,
            &format!("Pre-restore safety snapshot before restoring {backup_id}"),
            mode,
        )?
        .id
    };

    let mut created_files = 0;
    for (f, dst) in to_create {
        // Re-check after the snapshot: a file that appeared meanwhile is never overwritten.
        if dst.exists() {
            preserved_conflicts += 1;
            continue;
        }
        let src = backup_base.join(&f.relative_path);
        let bytes = fs::read(&src).map_err(|e| format!("Failed reading backup file {}: {e}", src.display()))?;
        atomic_write_verified(&dst, &bytes)?;
        created_files += 1;
    }

    append_journal(
        backup_storage_dir,
        "restore",
        mode,
        &format!(
            "Restored backup {backup_id} -> created {created_files}, preserved {preserved_conflicts} conflicts, skipped {skipped_identical} identical"
        ),
        "ok",
    );

    Ok(RestoreResult {
        backup_id: backup_id.to_string(),
        pre_restore_backup_id,
        created_files,
        preserved_conflicts,
        skipped_identical,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn fake(dir: &Path, id: &str, days_ago: i64, reason: &str) {
        fake_kind(dir, id, chrono::Duration::days(days_ago), reason, None);
    }

    fn fake_kind(dir: &Path, id: &str, age: chrono::Duration, reason: &str, kind: Option<&str>) {
        let d = dir.join(id);
        fs::create_dir_all(&d).unwrap();
        let m = BackupManifest {
            id: id.into(),
            created_at: (Utc::now() - age).to_rfc3339(),
            reason: reason.into(),
            source_root: String::new(),
            backup_dir: d.to_string_lossy().into_owned(),
            total_files: 0,
            total_bytes: 0,
            all_verified: true,
            files: vec![],
            kind: kind.map(String::from),
        };
        fs::write(d.join("backup-manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    }

    const ACC: &str = "11111111-2222-3333-4444-555555555555";

    fn rel(name: &str) -> String {
        format!("claude-code-sessions/{ACC}/org/{name}")
    }

    /// Live root with three pointers, backed up; then `a` is deleted (missing), `b` changed (different), `c` kept.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let t = tempfile::tempdir().unwrap();
        let live = t.path().join("live");
        let store = t.path().join("store");
        for (n, body) in [
            ("a.json", r#"{"title":"Chat A","lastActivityAt":1762700500000}"#),
            ("b.json", r#"{"title":"Chat B","lastActivityAt":2}"#),
            ("c.json", r#"{"lastActivityAt":3}"#),
        ] {
            let p = live.join(rel(n));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        let id = create_verified_backup(&live, &store, "test", "test").unwrap().id;
        fs::remove_file(live.join(rel("a.json"))).unwrap();
        fs::write(live.join(rel("b.json")), "changed").unwrap();
        (t, live, store, id)
    }

    fn backup_count(store: &Path) -> usize {
        list_backups(store).len()
    }

    #[test]
    fn preview_reports_missing_different_identical_and_writes_nothing() {
        let (_t, live, store, id) = setup();
        let before = backup_count(&store);
        let p = preview_restore(&store, &id, &live).unwrap();
        let by = |n: &str| p.files.iter().find(|f| f.relative_path == rel(n)).unwrap().clone();
        assert_eq!(p.files.len(), 3);
        let a = by("a.json");
        assert_eq!((a.status.as_str(), a.title.as_str(), a.last_activity_at, a.account_uuid.as_str()), ("missing", "Chat A", Some(1762700500000), ACC));
        assert_eq!(by("b.json").status, "different");
        let c = by("c.json");
        assert_eq!((c.status.as_str(), c.title.as_str()), ("identical", "c.json"));
        assert!(!live.join(rel("a.json")).exists());
        assert_eq!(backup_count(&store), before);
    }

    #[test]
    fn selective_restore_creates_only_the_chosen_file_and_keeps_different_ones() {
        let (_t, live, store, id) = setup();
        // A second missing file that is not chosen.
        fs::remove_file(live.join(rel("c.json"))).unwrap();
        let r = execute_restore(&store, &id, &live, "test", Some(&[rel("a.json"), rel("b.json")])).unwrap();
        assert_eq!((r.created_files, r.preserved_conflicts, r.skipped_identical), (1, 1, 0));
        assert!(!r.pre_restore_backup_id.is_empty());
        assert_eq!(fs::read_to_string(live.join(rel("a.json"))).unwrap(), r#"{"title":"Chat A","lastActivityAt":1762700500000}"#);
        assert_eq!(fs::read(live.join(rel("b.json"))).unwrap(), b"changed");
        assert!(!live.join(rel("c.json")).exists());
    }

    #[test]
    fn restore_rejects_paths_outside_the_manifest_before_writing() {
        let (_t, live, store, id) = setup();
        let before = backup_count(&store);
        let abs = live.join("x.json").to_string_lossy().into_owned();
        for bad in [r"..\x".to_string(), "claude-code-sessions/../x.json".into(), abs, "config.json".into()] {
            let err = execute_restore(&store, &id, &live, "test", Some(&[rel("a.json"), bad.clone()])).unwrap_err();
            assert!(err.contains("not a chat file"), "{bad}: {err}");
        }
        assert!(!live.join(rel("a.json")).exists());
        assert_eq!(backup_count(&store), before);
    }

    #[test]
    fn restore_with_nothing_missing_takes_no_pre_restore_backup() {
        let (_t, live, store, id) = setup();
        let before = backup_count(&store);
        let r = execute_restore(&store, &id, &live, "test", Some(&[])).unwrap();
        assert_eq!((r.created_files, r.pre_restore_backup_id.as_str()), (0, ""));
        assert_eq!(backup_count(&store), before);
        // Full restore brings `a` back (one snapshot); a second one has nothing missing: no snapshot.
        execute_restore(&store, &id, &live, "test", None).unwrap();
        assert_eq!(backup_count(&store), before + 1);
        let r = execute_restore(&store, &id, &live, "test", None).unwrap();
        assert_eq!((r.created_files, r.preserved_conflicts, r.skipped_identical, r.pre_restore_backup_id.as_str()), (0, 1, 2, ""));
        assert_eq!(backup_count(&store), before + 1);
        assert_eq!(fs::read(live.join(rel("b.json"))).unwrap(), b"changed");
    }

    #[test]
    fn prune_keeps_only_what_matters() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        // 12 old + 1 recent automatic: the 9 newest besides the new one survive, old08..old11 die
        for i in 0..12 {
            fake(p, &format!("backup-old{i:02}"), 30 + i, "Hesap değişimi");
        }
        fake(p, "backup-recent", 2, "Hesap değişimi");
        fake(p, "backup-manual", 100, "Manuel yedek");
        fake(p, "backup-manual2", 100, "Kullanıcı MANUEL Yedeği");
        fake(p, "initial-readonly-x", 100, "Initial");
        fake(p, "backup-new", 0, "Hesap değişimi");
        fs::create_dir_all(p.join("backup-nomanifest")).unwrap();
        fs::create_dir_all(p.join("other")).unwrap();

        prune_old_backups(p, "backup-new", false);

        let mut left: Vec<String> = fs::read_dir(p)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want: Vec<String> = (0..8).map(|i| format!("backup-old{i:02}")).collect();
        want.extend(
            ["backup-recent", "backup-manual", "backup-manual2", "initial-readonly-x", "backup-new", "backup-nomanifest", "other"]
                .map(String::from),
        );
        want.sort();
        assert_eq!(left, want);
    }

    #[test]
    fn prune_keeps_newest_20_live_refresh_backups_apart_from_full_ones() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        // 25 live-refresh backups, 15 minutes apart, all within today: only the newest 20 survive.
        for i in 0..25 {
            fake_kind(p, &format!("backup-live{i:02}"), chrono::Duration::minutes(15 * i), "x", Some(LIVE_REFRESH_BACKUP));
        }
        // 12 old full backups: the live ones take none of the newest-10 slots, so 10 survive.
        for i in 0..12 {
            fake(p, &format!("backup-full{i:02}"), 30 + i, "Hesap değişimi");
        }
        prune_old_backups(p, "backup-live00", true);
        let left: HashSet<String> = fs::read_dir(p).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let mut want: HashSet<String> = (0..20).map(|i| format!("backup-live{i:02}")).collect();
        want.extend((0..10).map(|i| format!("backup-full{i:02}")));
        assert_eq!(left, want);

        // A new full backup: the 20 live ones stay, the full ones keep 9 besides it.
        fake(p, "backup-fullnew", 0, "Hesap değişimi");
        prune_old_backups(p, "backup-fullnew", false);
        let left: HashSet<String> = fs::read_dir(p).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        want.remove("backup-full09");
        want.insert("backup-fullnew".into());
        assert_eq!(left, want);
    }
}
