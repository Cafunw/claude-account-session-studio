//! Multi-root path discovery for Claude Desktop (Classic, MSIX, Claude-Data, 3p)
//! and Claude Code CLI (`~/.claude`, `.claude.json`, `.credentials.json`).
//!
//! Never hardcodes a single path: discovers all candidate roots on Windows 11,
//! including `.user-data-left.*` redirected `Claude-Data` stores, MSIX packages,
//! and classic `%APPDATA%\Claude`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    /// `%LOCALAPPDATA%\Claude-Data\claude_<pkg>` (modern MSIX externalized userData)
    ClaudeDataExternal,
    /// `%LOCALAPPDATA%\Packages\Claude_<pkg>\LocalCache\Roaming\Claude`
    MsixVirtualized,
    /// `%APPDATA%\Claude` (classic Squirrel / unpackaged installer)
    ClassicRoaming,
    /// `%LOCALAPPDATA%\Claude-3p`
    Enterprise3p,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDataRoot {
    pub path: String,
    pub sessions_dir: String,
    pub kind: InstallKind,
    pub has_sessions_dir: bool,
    pub has_config_json: bool,
    pub last_known_account_uuid: Option<String>,
    pub account_count: usize,
    pub session_count: usize,
    pub is_active_root: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    pub active_data_root: Option<DiscoveredDataRoot>,
    pub all_discovered_roots: Vec<DiscoveredDataRoot>,
    pub cli_config_home: String,
    pub cli_global_config: String,
    pub cli_credentials_path: String,
    pub cli_projects_dir: String,
    pub cli_transcript_count: usize,
    pub desktop_exe_path: Option<String>,
    pub desktop_running: bool,
    pub backup_storage_dir: String,
}

/// Read `lastKnownAccountUuid` from a Claude Desktop `config.json` file.
pub fn read_last_known_account(config_json_path: &Path) -> Option<String> {
    let raw = fs::read_to_string(config_json_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    val.get("lastKnownAccountUuid")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn dir_newest_mtime(dir: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    for rel in ["config.json", "claude_desktop_config.json", "Local State"] {
        if let Ok(meta) = fs::metadata(dir.join(rel)) {
            if let Ok(m) = meta.modified() {
                if m > newest {
                    newest = m;
                }
            }
        }
    }
    let sessions = dir.join("claude-code-sessions");
    if let Ok(accs) = fs::read_dir(&sessions) {
        for acc in accs.flatten() {
            if let Ok(orgs) = fs::read_dir(acc.path()) {
                for org in orgs.flatten() {
                    if let Ok(files) = fs::read_dir(org.path()) {
                        for f in files.flatten() {
                            if f.file_name().to_string_lossy().starts_with("local_") {
                                if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                                    if m > newest {
                                        newest = m;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    newest
}

fn count_accounts_and_sessions(sessions_dir: &Path) -> (usize, usize) {
    if !sessions_dir.is_dir() {
        return (0, 0);
    }
    let mut accounts = 0;
    let mut sessions = 0;
    if let Ok(acc_entries) = fs::read_dir(sessions_dir) {
        for acc in acc_entries.flatten() {
            if !acc.path().is_dir() {
                continue;
            }
            accounts += 1;
            if let Ok(org_entries) = fs::read_dir(acc.path()) {
                for org in org_entries.flatten() {
                    if !org.path().is_dir() {
                        continue;
                    }
                    if let Ok(files) = fs::read_dir(org.path()) {
                        for f in files.flatten() {
                            let name = f.file_name().to_string_lossy().into_owned();
                            if name.starts_with("local_") && name.ends_with(".json") {
                                sessions += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    (accounts, sessions)
}

/// Discover all real Claude Desktop data roots on the host Windows 11 system
/// (read-only).
pub fn discover_real_roots() -> Vec<DiscoveredDataRoot> {
    let mut candidates: Vec<(PathBuf, InstallKind)> = Vec::new();

    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let localappdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);

    // 1. Classic %APPDATA%\Claude
    if let Some(ref roam) = appdata {
        let p = roam.join("Claude");
        if p.is_dir() {
            candidates.push((p, InstallKind::ClassicRoaming));
        }
    }

    if let Some(ref local) = localappdata {
        // 2. Modern MSIX externalized %LOCALAPPDATA%\Claude-Data\claude_*
        let claude_data = local.join("Claude-Data");
        if let Ok(entries) = fs::read_dir(&claude_data) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    candidates.push((p, InstallKind::ClaudeDataExternal));
                }
            }
        }

        // 3. MSIX virtualized %LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude
        let packages = local.join("Packages");
        if let Ok(entries) = fs::read_dir(&packages) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("claude_") || name.starts_with("anthropic.claude_") {
                    let p = e.path().join("LocalCache").join("Roaming").join("Claude");
                    if p.is_dir() {
                        candidates.push((p, InstallKind::MsixVirtualized));
                    }
                }
            }
        }

        // 4. Enterprise / 3p %LOCALAPPDATA%\Claude-3p
        let p3 = local.join("Claude-3p");
        if p3.is_dir() && p3.join("claude-code-sessions").is_dir() {
            candidates.push((p3, InstallKind::Enterprise3p));
        }
    }

    // Rank candidates by whether they hold sessions/config and newest activity
    let mut scored: Vec<(DiscoveredDataRoot, SystemTime, bool)> = Vec::new();
    for (path, kind) in candidates {
        let sessions_dir = path.join("claude-code-sessions");
        let config_json = path.join("config.json");
        let has_sessions_dir = sessions_dir.is_dir();
        let has_config_json = config_json.is_file();
        let last_known_account_uuid = read_last_known_account(&config_json);
        let (account_count, session_count) = count_accounts_and_sessions(&sessions_dir);
        let mtime = dir_newest_mtime(&path);
        let meaningful = has_sessions_dir || has_config_json || session_count > 0;

        scored.push((
            DiscoveredDataRoot {
                path: path.to_string_lossy().into_owned(),
                sessions_dir: sessions_dir.to_string_lossy().into_owned(),
                kind,
                has_sessions_dir,
                has_config_json,
                last_known_account_uuid,
                account_count,
                session_count,
                is_active_root: false,
            },
            mtime,
            meaningful,
        ));
    }

    scored.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.1.cmp(&a.1)));
    let mut out: Vec<DiscoveredDataRoot> = scored.into_iter().map(|(r, _, _)| r).collect();
    if let Some(first) = out.first_mut() {
        first.is_active_root = true;
    }
    out
}

/// Count `.jsonl` transcripts in `~/.claude/projects` (read-only).
pub fn count_cli_transcripts(projects_dir: &Path) -> usize {
    if !projects_dir.is_dir() {
        return 0;
    }
    let mut count = 0;
    if let Ok(projs) = fs::read_dir(projects_dir) {
        for p in projs.flatten() {
            if let Ok(files) = fs::read_dir(p.path()) {
                for f in files.flatten() {
                    if f.file_name().to_string_lossy().ends_with(".jsonl") {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Locate Claude Desktop executable (`Claude.exe`), carefully excluding Claude Code CLI.
pub fn find_desktop_executable() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let direct = [
        local.join("AnthropicClaude").join("claude.exe"),
        local.join("Programs").join("Claude").join("Claude.exe"),
        local.join("Microsoft").join("WindowsApps").join("Claude.exe"),
    ];
    for c in &direct {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    // Check WindowsApps MSIX packages
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let winapps = PathBuf::from(pf).join("WindowsApps");
        if let Ok(entries) = fs::read_dir(winapps) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("claude_") {
                    let exe = e.path().join("app").join("Claude.exe");
                    if exe.is_file() {
                        return Some(exe);
                    }
                }
            }
        }
    }
    None
}

/// Check if Claude Desktop (Electron app, NOT Claude Code CLI) is currently running.
pub fn is_claude_desktop_running(active_data_root: Option<&Path>) -> bool {
    if let Some(root) = active_data_root {
        for lock_rel in [
            "lockfile",
            "SingletonLock",
            "Network/Cookies",
            "Local Storage/leveldb/LOCK",
        ] {
            if crate::dpapi::is_file_locked(&root.join(lock_rel)) {
                return true;
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == -1 as _ || snap.is_null() {
                return false;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = false;
            if Process32FirstW(snap, &mut entry) != 0 {
                loop {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
                    if name == "claude.exe" {
                        let h = OpenProcess(
                            PROCESS_QUERY_LIMITED_INFORMATION,
                            0,
                            entry.th32ProcessID,
                        );
                        if !h.is_null() {
                            let mut buf = [0u16; 1024];
                            let mut sz = buf.len() as u32;
                            if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut sz) != 0 {
                                let full = String::from_utf16_lossy(&buf[..sz as usize])
                                    .to_ascii_lowercase()
                                    .replace('/', "\\");
                                let is_cli = full.contains("\\.local\\")
                                    || full.contains("\\claude-code\\")
                                    || full.contains("\\claude-code-vm\\")
                                    || full.contains("\\roaming\\npm\\")
                                    || full.contains("\\anthropic.claudecode_");
                                if !is_cli {
                                    found = true;
                                }
                            }
                            CloseHandle(h);
                        }
                    }
                    if found || Process32NextW(snap, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
            found
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
