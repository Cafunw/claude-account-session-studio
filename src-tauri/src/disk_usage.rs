//! How much disk the Claude data uses, for the read-only "Disk" page. Only directory listings and
//! file metadata are read: no file contents, no links or junctions followed, nothing deleted.

use crate::backup::{list_backups, LIVE_REFRESH_BACKUP};
use crate::AppState;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    /// Newest file modification time (ms); 0 with no files.
    pub last_modified: i64,
}

impl Size {
    fn add(&mut self, o: &Size) {
        self.bytes += o.bytes;
        self.files += o.files;
        self.last_modified = self.last_modified.max(o.last_modified);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    /// Folder name under the CLI projects dir.
    pub folder: String,
    /// The project's working folder when the spending cache knows it, else `folder`.
    pub path: String,
    pub size: Size,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupCount {
    pub bytes: u64,
    pub count: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupUsage {
    /// Everything in the backup folder (also the journal and unrecognised folders).
    pub total: Size,
    pub full: BackupCount,
    pub live_refresh: BackupCount,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub transcripts: Size,
    /// Largest first.
    pub projects: Vec<ProjectUsage>,
    pub backups: BackupUsage,
    pub pointers: Size,
    pub app: Size,
}

fn mtime_ms(m: &fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

/// Sum of the files under `dir`; a missing folder is zero. Links and junctions (Windows reports
/// both as symlinks) are skipped, never followed.
fn dir_size(dir: &Path) -> Size {
    let mut out = Size::default();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            out.add(&dir_size(&e.path()));
        } else if let Ok(m) = e.metadata() {
            // DirEntry metadata comes with the directory listing on Windows: no extra open.
            out.add(&Size { bytes: m.len(), files: 1, last_modified: mtime_ms(&m) });
        }
    }
    out
}

/// One row per project folder (loose files count toward the total only), largest first.
fn transcripts(projects_dir: &Path, cwds: &std::collections::HashMap<String, String>) -> (Size, Vec<ProjectUsage>) {
    let (mut total, mut rows) = (Size::default(), Vec::new());
    for e in fs::read_dir(projects_dir).into_iter().flatten().flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            let size = dir_size(&e.path());
            total.add(&size);
            let folder = e.file_name().to_string_lossy().into_owned();
            let path = cwds.get(&folder).cloned().unwrap_or_else(|| folder.clone());
            rows.push(ProjectUsage { folder, path, size });
        } else if let Ok(m) = e.metadata() {
            total.add(&Size { bytes: m.len(), files: 1, last_modified: mtime_ms(&m) });
        }
    }
    rows.sort_by(|a, b| b.size.bytes.cmp(&a.size.bytes).then_with(|| a.folder.cmp(&b.folder)));
    (total, rows)
}

fn backups(storage: &Path) -> BackupUsage {
    let mut out = BackupUsage { total: dir_size(storage), ..Default::default() };
    for m in list_backups(storage) {
        // The id is the folder name; anything else in a manifest is not trusted as a path.
        if Path::new(&m.id).file_name().is_none_or(|n| n != m.id.as_str()) {
            continue;
        }
        let bytes = dir_size(&storage.join(&m.id)).bytes;
        let c = if m.kind.as_deref() == Some(LIVE_REFRESH_BACKUP) { &mut out.live_refresh } else { &mut out.full };
        c.bytes += bytes;
        c.count += 1;
    }
    out
}

fn active_pointers_dir() -> Option<PathBuf> {
    crate::paths::discover_real_roots().first().map(|r| PathBuf::from(&r.path).join("claude-code-sessions"))
}

fn measure(projects_dir: &Path, storage: &Path, pointers: Option<&Path>, studio_dir: &Path) -> DiskUsage {
    let (transcripts, projects) = transcripts(projects_dir, &crate::spending::cached_project_cwds(projects_dir));
    DiskUsage { transcripts, projects, backups: backups(storage), pointers: pointers.map(dir_size).unwrap_or_default(), app: dir_size(studio_dir) }
}

#[tauri::command(async)]
pub fn get_disk_usage(state: State<'_, AppState>) -> DiskUsage {
    measure(&crate::chat_text::cli_projects_dir(), &state.backup_storage_dir, active_pointers_dir().as_deref(), &state.studio_dir)
}

/// Opens one of the measured folders in File Explorer. The folder comes from a fixed kind, never
/// from a path the page sends.
#[tauri::command(async)]
pub fn reveal_disk_folder(kind: String, state: State<'_, AppState>) -> Result<(), String> {
    let dir = match kind.as_str() {
        "transcripts" => Some(crate::chat_text::cli_projects_dir()),
        "backups" => Some(state.backup_storage_dir.clone()),
        "pointers" => active_pointers_dir(),
        "app" => Some(state.studio_dir.clone()),
        _ => None,
    }
    .filter(|d| d.is_dir())
    .ok_or_else(|| crate::i18n::t("Klasör bulunamadı.", "Folder not found.").to_string())?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(&dir)
            .spawn()
            .map_err(|e| crate::i18n::t(format!("Gezgin açılamadı: {e}"), format!("Couldn't open File Explorer: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Err(crate::i18n::t("Yalnızca Windows'ta desteklenir.", "Only supported on Windows.").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn file(path: &Path, len: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; len]).unwrap();
    }

    /// A directory link at `link` to `target`: a symlink, else (Windows) a junction. False if neither works here.
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
                || std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().is_ok_and(|o| o.status.success())
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    #[test]
    fn sums_per_project_largest_first_with_known_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        file(&p.join("C--a/one.jsonl"), 100);
        file(&p.join("C--a/one/subagents/agent-1.jsonl"), 50);
        file(&p.join("C--b/two.jsonl"), 300);
        file(&p.join("loose.txt"), 7);
        fs::create_dir_all(p.join("empty")).unwrap();
        let cwds = HashMap::from([("C--a".to_string(), "C:\\a".to_string())]);
        let (total, rows) = transcripts(p, &cwds);
        assert_eq!((total.bytes, total.files), (457, 4));
        assert!(total.last_modified > 0);
        let got: Vec<_> = rows.iter().map(|r| (r.folder.as_str(), r.path.as_str(), r.size.bytes, r.size.files)).collect();
        assert_eq!(got, [("C--b", "C--b", 300, 1), ("C--a", "C:\\a", 150, 2), ("empty", "empty", 0, 0)]);
        assert_eq!(rows[2].size.last_modified, 0);
    }

    #[test]
    fn links_and_junctions_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        file(&outside.join("big.bin"), 1000);
        let root = tmp.path().join("root");
        file(&root.join("proj/a.jsonl"), 10);
        if !link_dir(&outside, &root.join("proj/link")) || !link_dir(&outside, &root.join("linked-proj")) {
            eprintln!("skipped: no directory link could be created here");
            return;
        }
        assert_eq!(dir_size(&root).bytes, 10);
        let (total, rows) = transcripts(&root, &HashMap::new());
        assert_eq!((total.bytes, rows.len()), (10, 1));
        // The target is untouched.
        assert_eq!(fs::metadata(outside.join("big.bin")).unwrap().len(), 1000);
    }

    #[test]
    fn missing_folders_count_as_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("gone");
        let d = measure(&gone, &gone, None, &gone);
        assert_eq!((d.transcripts, d.pointers, d.app, d.backups.total), Default::default());
        assert!(d.projects.is_empty());
        assert_eq!((d.backups.full, d.backups.live_refresh), Default::default());
    }

    #[test]
    fn backups_split_into_full_and_live_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let s = tmp.path();
        let manifest = |id: &str, kind: Option<&str>| {
            let m = serde_json::json!({
                "id": id, "createdAt": "2026-01-01T00:00:00Z", "reason": "", "sourceRoot": "", "backupDir": "",
                "totalFiles": 0, "totalBytes": 0, "allVerified": true, "files": [], "kind": kind,
            });
            file(&s.join(id).join("backup-manifest.json"), 0);
            fs::write(s.join(id).join("backup-manifest.json"), m.to_string()).unwrap();
            fs::metadata(s.join(id).join("backup-manifest.json")).unwrap().len()
        };
        let m1 = manifest("full-1", None);
        file(&s.join("full-1/claude-code-sessions/x.json"), 200);
        let m2 = manifest("live-1", Some(LIVE_REFRESH_BACKUP));
        file(&s.join("live-1/claude-code-sessions/y.json"), 20);
        // A manifest whose id is not its own folder name is not measured.
        manifest("odd", None);
        let odd = s.join("odd/backup-manifest.json");
        let text = fs::read_to_string(&odd).unwrap().replace("\"odd\"", "\"../full-1\"");
        fs::write(&odd, &text).unwrap();
        file(&s.join("journal.jsonl"), 5);
        let b = backups(s);
        assert_eq!(b.full, BackupCount { bytes: m1 + 200, count: 1 });
        assert_eq!(b.live_refresh, BackupCount { bytes: m2 + 20, count: 1 });
        assert_eq!(b.total.bytes, m1 + 200 + m2 + 20 + text.len() as u64 + 5);
    }

    #[test]
    fn ten_thousand_files_in_a_few_seconds() {
        let tmp = tempfile::tempdir().unwrap();
        for p in 0..100 {
            for f in 0..100 {
                file(&tmp.path().join(format!("p{p}/{f}.jsonl")), 1);
            }
        }
        let t = std::time::Instant::now();
        let (total, rows) = transcripts(tmp.path(), &HashMap::new());
        assert_eq!((total.files, rows.len()), (10_000, 100));
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "{:?}", t.elapsed());
    }
}
