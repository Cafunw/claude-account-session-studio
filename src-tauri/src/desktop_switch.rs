//! Automated Claude Desktop process lifecycle (graceful close -> wait for lock release ->
//! swap Electron auth & CLI credentials -> sync all sessions -> relaunch Claude Desktop).
//!
//! Supports Windows 11 MSIX externalized `Claude-Data\claude_*` directories; saved logins are DPAPI-encrypted.

use crate::dpapi::{atomic_write_verified, dpapi_decrypt, dpapi_encrypt, is_file_locked};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Files inside Claude Desktop's `userData` directory (`Claude-Data\claude_*` or `%APPDATA%\Claude`)
/// that carry the logged-in account's Electron session and DPAPI key.
pub const DESKTOP_AUTH_FILES: &[&str] = &[
    "Local State",
    "config.json",
    "Preferences",
    "ant-did",
    "ant-device-registry.json",
    "DIPS",
    "DIPS-wal",
    "DIPS-shm",
    "Network/Cookies",
    "Network/Cookies-journal",
    "Network/Cookies-wal",
    "Network/Cookies-shm",
    "Network/Network Persistent State",
    "Network/TransportSecurity",
    "Network/Trust Tokens",
    "Network/Trust Tokens-journal",
    "Cookies",
    "Cookies-journal",
    "Cookies-wal",
    "Cookies-shm",
];

/// Session files cleared when preparing Claude Desktop for a brand-new account sign-in.
/// Preserves `Local State` (DPAPI master key) and `claude_desktop_config.json` (MCP & sidebar groups).
pub const SESSION_CLEAR_FILES: &[&str] = &[
    "config.json",
    "DIPS",
    "DIPS-wal",
    "DIPS-shm",
    "Network/Cookies",
    "Network/Cookies-journal",
    "Network/Cookies-wal",
    "Network/Cookies-shm",
    "Network/Network Persistent State",
    "Network/TransportSecurity",
    "Network/Trust Tokens",
    "Network/Trust Tokens-journal",
    "Cookies",
    "Cookies-journal",
    "Cookies-wal",
    "Cookies-shm",
];

/// Directories in Claude Desktop `userData` that hold `claude.ai` origin storage.
pub const DESKTOP_AUTH_DIRS: &[&str] = &[
    "Local Storage",
    "Session Storage",
    "IndexedDB",
    "WebStorage",
];

pub fn slot_vault_dir(studio_dir: &Path, account_uuid: &str) -> PathBuf {
    studio_dir.join("slots").join(account_uuid)
}

pub fn slot_desktop_auth_dir(studio_dir: &Path, account_uuid: &str) -> PathBuf {
    slot_vault_dir(studio_dir, account_uuid).join("desktop_auth")
}

/// Check if an account slot has a captured Claude Desktop login (`Network/Cookies` or `Local Storage`).
pub fn has_saved_desktop_login(studio_dir: &Path, account_uuid: &str) -> bool {
    let dir = slot_desktop_auth_dir(studio_dir, account_uuid);
    if !dir.is_dir() {
        return false;
    }
    for rel in ["Network/Cookies", "Cookies"] {
        let p = dir.join(rel);
        if p.is_file() && fs::metadata(&p).map_or(0, |m| m.len()) > 0 {
            return true;
        }
    }
    dir.join("Local Storage").join("leveldb").is_dir()
}

/// Check if `data_root` currently has an active login.
pub fn data_root_has_live_auth(data_root: &Path) -> bool {
    for rel in ["Network/Cookies", "Cookies"] {
        let p = data_root.join(rel);
        if p.is_file() && fs::metadata(&p).map_or(0, |m| m.len()) > 0 {
            return true;
        }
    }
    data_root.join("Local Storage").join("leveldb").is_dir()
}

/// Check if any critical file in `data_root` is currently locked by Claude Desktop.
pub fn is_data_root_locked(data_root: &Path) -> bool {
    for rel in [
        "lockfile",
        "SingletonLock",
        "Network/Cookies",
        "Cookies",
        "Local Storage/leveldb/LOCK",
    ] {
        if is_file_locked(&data_root.join(rel)) {
            return true;
        }
    }
    false
}

/// Find all running Claude Desktop PIDs on Windows (including Electron helper processes).
#[cfg(windows)]
pub fn find_claude_desktop_pids() -> Vec<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == -1 as _ || snap.is_null() {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
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
                            // Match Claude Desktop (MSIX WindowsApps\Claude_*, AnthropicClaude, Programs\Claude)
                            // and Desktop-spawned claude-code child processes inside Claude-Data
                            let is_desktop = full.contains("\\windowsapps\\claude_")
                                || full.contains("\\anthropicclaude\\")
                                || full.contains("\\programs\\claude\\")
                                || full.contains("\\claude-data\\");
                            if is_desktop {
                                out.push(entry.th32ProcessID);
                            }
                        }
                        CloseHandle(h);
                    }
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
pub fn find_claude_desktop_pids() -> Vec<u32> {
    Vec::new()
}

#[cfg(windows)]
fn send_wm_close_to_pids(pids: &[u32]) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let target_pids = &*(lparam as *const Vec<u32>);
        let mut win_pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut win_pid);
        if target_pids.contains(&win_pid) {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
        1
    }

    let pid_vec = pids.to_vec();
    unsafe {
        EnumWindows(Some(enum_cb), std::ptr::addr_of!(pid_vec) as LPARAM);
    }
}

#[cfg(windows)]
fn terminate_pid_force(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Automatically close Claude Desktop and wait until all its processes and file locks release.
pub fn close_claude_desktop_and_wait(data_root: &Path, timeout: Duration) -> Result<bool, String> {
    let pids = find_claude_desktop_pids();
    let locked = is_data_root_locked(data_root);
    if pids.is_empty() && !locked {
        return Ok(false);
    }

    #[cfg(windows)]
    if !pids.is_empty() {
        // 1. Graceful WM_CLOSE so Electron flushes SQLite Cookies and LevelDB cleanly
        send_wm_close_to_pids(&pids);
        let graceful = Duration::from_millis(2200);
        let start = Instant::now();
        while start.elapsed() < graceful {
            if find_claude_desktop_pids().is_empty() && !is_data_root_locked(data_root) {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // 2. Force-terminate any remaining Claude Desktop processes
        for pid in find_claude_desktop_pids() {
            terminate_pid_force(pid);
        }
    }

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if find_claude_desktop_pids().is_empty() && !is_data_root_locked(data_root) {
            // Brief extra settle delay for Windows NTFS handle cleanup
            std::thread::sleep(Duration::from_millis(200));
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if is_data_root_locked(data_root) {
        return Err(crate::i18n::t(
            "Claude Desktop dosya kilitleri serbest bırakılamadı; lütfen açık pencereyi kapatıp tekrar deneyin.",
            "Claude Desktop didn't release its file locks; close its open window and try again.",
        )
        .to_string());
    }
    Ok(true)
}

/// Automatically launch Claude Desktop on Windows 11 (supports MSIX `Claude_pzs8sxrjxfjjc!Claude`
/// via `explorer.exe shell:AppsFolder\...` as well as standard `.exe` installs).
pub fn launch_claude_desktop() -> Result<bool, String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // 1. Check if MSIX package `Claude_*` exists under %LOCALAPPDATA%\Packages
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            let packages = local.join("Packages");
            if let Ok(entries) = fs::read_dir(packages) {
                for entry in entries.flatten() {
                    let pkg_name = entry.file_name().to_string_lossy().into_owned();
                    let lower = pkg_name.to_ascii_lowercase();
                    if lower.starts_with("claude_") || lower.starts_with("anthropic.claude_") {
                        let app_user_model_id = format!("shell:AppsFolder\\{pkg_name}!Claude");
                        if Command::new("explorer.exe")
                            .arg(&app_user_model_id)
                            .creation_flags(CREATE_NO_WINDOW)
                            .spawn()
                            .is_ok()
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }

        // 2. Fallback to direct executable path
        if let Some(exe) = crate::paths::find_desktop_executable() {
            let mut cmd = Command::new(&exe);
            if let Some(parent) = exe.parent() {
                cmd.current_dir(parent);
            }
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.spawn()
                .map(|_| true)
                .map_err(|e| crate::i18n::t(format!("Claude Desktop başlatılamadı: {e}"), format!("Couldn't start Claude Desktop: {e}")))
        } else {
            Err(crate::i18n::t("Claude Desktop uygulaması bulunamadı.", "Claude Desktop app not found.").to_string())
        }
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

fn copy_dir_recursive_unlocked(src: &Path, dst: &Path) -> io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ftype = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ftype.is_dir() {
            copy_dir_recursive_unlocked(&src_path, &dst_path)?;
        } else if ftype.is_file() {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.eq_ignore_ascii_case("LOCK") && is_file_locked(&src_path) {
                continue;
            }
            let _ = fs::copy(&src_path, &dst_path);
        }
    }
    Ok(())
}

fn copy_desktop_auth_tree(src_root: &Path, dst_root: &Path) -> io::Result<()> {
    fs::create_dir_all(dst_root)?;
    for &rel in DESKTOP_AUTH_FILES {
        let src = src_root.join(rel);
        let dst = dst_root.join(rel);
        if src.is_file() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        } else if dst.is_file() {
            let _ = fs::remove_file(&dst);
        }
    }
    for &rel in DESKTOP_AUTH_DIRS {
        let src = src_root.join(rel);
        let dst = dst_root.join(rel);
        if dst.is_dir() {
            let _ = fs::remove_dir_all(&dst);
        }
        if src.is_dir() {
            copy_dir_recursive_unlocked(&src, &dst)?;
        }
    }
    Ok(())
}

/// Clear only the active session cookies in `data_root` so Claude Desktop opens to a clean
/// sign-in screen for adding a new account, while preserving `Local State`, `claude_desktop_config.json`,
/// and all `claude-code-sessions`!
pub fn clear_main_desktop_session_for_new_login(data_root: &Path) -> Result<(), String> {
    for &rel in SESSION_CLEAR_FILES {
        let p = data_root.join(rel);
        if p.is_file() {
            let _ = fs::remove_file(&p);
        }
    }
    for &rel in DESKTOP_AUTH_DIRS {
        let p = data_root.join(rel);
        if p.is_dir() {
            let _ = fs::remove_dir_all(&p);
        }
    }
    Ok(())
}

/// Capture the currently active account's Desktop auth tree AND CLI `.credentials.json` / `.claude.json`
/// into `%LOCALAPPDATA%\ClaudeSessionStudio\slots\<account_uuid>\`.
pub fn capture_active_account_state(
    data_root: &Path,
    studio_dir: &Path,
    account_uuid: &str,
) -> Result<(), String> {
    if account_uuid.trim().is_empty() {
        return Ok(());
    }
    let slot_dir = slot_vault_dir(studio_dir, account_uuid);
    let desktop_auth_dst = slot_desktop_auth_dir(studio_dir, account_uuid);
    fs::create_dir_all(&desktop_auth_dst).map_err(|e| e.to_string())?;

    // 1. Copy Electron Desktop auth files if unlocked
    if data_root_has_live_auth(data_root) && !is_data_root_locked(data_root) {
        copy_desktop_auth_tree(data_root, &desktop_auth_dst)
            .map_err(|e| crate::i18n::t(format!("Desktop auth kopyalanamadı: {e}"), format!("Couldn't copy the Desktop login: {e}")))?;
    }

    // 2. Capture CLI `.credentials.json` and `.claude.json` (`oauthAccount`) if they match this account
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let cli_global = home.join(".claude.json");
        let cli_creds = home.join(".claude").join(".credentials.json");

        if let Ok(global_text) = fs::read_to_string(&cli_global) {
            if let Ok(val) = serde_json::from_str::<Value>(&global_text) {
                let cli_uuid = val
                    .pointer("/oauthAccount/accountUuid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if cli_uuid.eq_ignore_ascii_case(account_uuid) {
                    if let Some(oa) = val.get("oauthAccount") {
                        let oa_json = json!({ "oauthAccount": oa }).to_string();
                        if let Ok(enc_cfg) = dpapi_encrypt(&oa_json) {
                            let _ = atomic_write_verified(
                                &slot_dir.join("cli_oauth_account.dpapi"),
                                enc_cfg.as_bytes(),
                            );
                        }
                    }
                    if let Ok(creds_text) = fs::read_to_string(&cli_creds) {
                        if creds_text.contains("accessToken") {
                            if let Ok(enc_creds) = dpapi_encrypt(&creds_text) {
                                let _ = atomic_write_verified(
                                    &slot_dir.join("cli_credentials.dpapi"),
                                    enc_creds.as_bytes(),
                                );
                            }
                            let _ = crate::accounts::store_dpapi_credentials(
                                studio_dir,
                                account_uuid,
                                &creds_text,
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// The vault copy (`credentials/<uuid>.dpapi`) can hold tokens this app refreshed after the slot was
/// saved, which revoked the slot's refresh token, so restore whichever expires later (vault on a tie).
pub fn pick_cli_creds_to_restore<'a>(slot: &'a str, vault: Option<&'a str>) -> &'a str {
    let expiry = |s: &str| -> Option<i64> {
        let v: Value = serde_json::from_str(s).ok()?;
        let o = v.get("claudeAiOauth")?;
        o.get("accessToken")?.as_str().filter(|t| !t.is_empty())?;
        Some(o.get("expiresAt").and_then(|e| e.as_i64()).unwrap_or(0))
    };
    match (vault, vault.and_then(expiry)) {
        (Some(v), Some(ve)) if expiry(slot).map_or(true, |se| ve >= se) => v,
        _ => slot,
    }
}

/// Restore `target_account_uuid`'s saved Desktop auth tree into `data_root`, update `config.json`
/// (`lastKnownAccountUuid`), and restore its CLI `.credentials.json` + `.claude.json` (`oauthAccount`).
/// Returns `Ok(true)` if a full Desktop login state was restored, or `Ok(false)` if the slot had no
/// saved Desktop cookies yet (so Claude Desktop will prompt to sign in once).
pub fn restore_target_account_state(
    data_root: &Path,
    studio_dir: &Path,
    target_account_uuid: &str,
) -> Result<bool, String> {
    let slot_dir = slot_vault_dir(studio_dir, target_account_uuid);
    let desktop_auth_src = slot_desktop_auth_dir(studio_dir, target_account_uuid);
    let has_desktop = has_saved_desktop_login(studio_dir, target_account_uuid);

    if has_desktop {
        // Preserve Local State if slot somehow lacks one
        let keep_local_state = !desktop_auth_src.join("Local State").is_file()
            && data_root.join("Local State").is_file();
        let saved_ls = if keep_local_state {
            fs::read(data_root.join("Local State")).ok()
        } else {
            None
        };

        copy_desktop_auth_tree(&desktop_auth_src, data_root)
            .map_err(|e| {
                crate::i18n::t(
                    format!("Hedef hesap Desktop oturumu geri yüklenemedi: {e}"),
                    format!("Couldn't restore the target account's Desktop session: {e}"),
                )
            })?;

        if let Some(ls_bytes) = saved_ls {
            let _ = atomic_write_verified(&data_root.join("Local State"), &ls_bytes);
        }
    } else {
        // Target account doesn't have saved Desktop cookies yet -> clear cookies so Desktop opens to login
        clear_main_desktop_session_for_new_login(data_root)?;
    }

    // Always ensure `lastKnownAccountUuid` in `data_root/config.json` points to `target_account_uuid`
    let cfg_path = data_root.join("config.json");
    let mut cfg_val: Value = if cfg_path.is_file() {
        fs::read_to_string(&cfg_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}))
    } else {
        json!({
            "scale": 0,
            "locale": "en-US",
            "userThemeMode": "dark"
        })
    };
    if let Some(obj) = cfg_val.as_object_mut() {
        obj.insert("lastKnownAccountUuid".to_string(), json!(target_account_uuid));
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&cfg_val) {
        let _ = atomic_write_verified(&cfg_path, &bytes);
    }

    // Also restore CLI `.credentials.json` and `.claude.json` (`oauthAccount`) if available
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let cli_creds_dpapi = slot_dir.join("cli_credentials.dpapi");
        if let Ok(enc) = fs::read_to_string(&cli_creds_dpapi) {
            if let Ok(plain_creds) = dpapi_decrypt(&enc) {
                let vault = crate::accounts::load_dpapi_credentials(studio_dir, target_account_uuid).ok();
                let chosen = pick_cli_creds_to_restore(&plain_creds, vault.as_deref());
                let cli_creds_path = home.join(".claude").join(".credentials.json");
                let _ = atomic_write_verified(&cli_creds_path, chosen.as_bytes());
            }
        }

        let cli_oa_dpapi = slot_dir.join("cli_oauth_account.dpapi");
        if let Ok(enc) = fs::read_to_string(&cli_oa_dpapi) {
            if let Ok(plain_oa) = dpapi_decrypt(&enc) {
                if let Ok(oa_wrapper) = serde_json::from_str::<Value>(&plain_oa) {
                    if let Some(oa_val) = oa_wrapper.get("oauthAccount") {
                        let cli_global_path = home.join(".claude.json");
                        let mut global_val: Value = fs::read_to_string(&cli_global_path)
                            .ok()
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_else(|| json!({}));
                        if let Some(obj) = global_val.as_object_mut() {
                            obj.insert("oauthAccount".to_string(), oa_val.clone());
                        }
                        if let Ok(bytes) = serde_json::to_vec_pretty(&global_val) {
                            let _ = atomic_write_verified(&cli_global_path, &bytes);
                        }
                    }
                }
            }
        }
    }

    Ok(has_desktop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_picks_the_copy_that_expires_later() {
        let slot = r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"old","expiresAt":1000}}"#;
        let newer = r#"{"claudeAiOauth":{"accessToken":"b","refreshToken":"new","expiresAt":2000}}"#;
        let older = r#"{"claudeAiOauth":{"accessToken":"c","refreshToken":"x","expiresAt":500}}"#;
        let tie = r#"{"claudeAiOauth":{"accessToken":"d","refreshToken":"y","expiresAt":1000}}"#;
        assert_eq!(pick_cli_creds_to_restore(slot, Some(newer)), newer);
        assert_eq!(pick_cli_creds_to_restore(slot, Some(older)), slot);
        assert_eq!(pick_cli_creds_to_restore(slot, Some(tie)), tie);
        assert_eq!(pick_cli_creds_to_restore(slot, None), slot);
        // An unusable vault copy never replaces the slot; a usable one beats an unusable slot.
        assert_eq!(pick_cli_creds_to_restore(slot, Some("{")), slot);
        assert_eq!(pick_cli_creds_to_restore(slot, Some(r#"{"claudeAiOauth":{"accessToken":""}}"#)), slot);
        assert_eq!(pick_cli_creds_to_restore("{", Some(older)), older);
    }
}
