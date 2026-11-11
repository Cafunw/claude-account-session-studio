//! Self-update from GitHub Releases through the signed Tauri updater. Network and parse errors stay
//! silent (journaled at most once an hour); with auto-update off the app never contacts GitHub by itself.
use crate::accounts::save_studio_config;
use crate::backup::append_journal;
use crate::i18n::t;
use crate::{build_dashboard_state, AppState, FullDashboardPayload};
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(6 * 3600);
const ERROR_LOG_GAP: Duration = Duration::from_secs(3600);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    available: bool,
    version: Option<String>,
    notes: Option<String>,
    current_version: String,
}

#[derive(Debug, PartialEq)]
enum Action {
    Install,
    Announce,
    Nothing,
}

/// The startup check installs at once; later finds are announced once per version.
fn decide(first: bool, found: Option<&str>, announced: Option<&str>) -> Action {
    match found {
        None => Action::Nothing,
        Some(_) if first => Action::Install,
        Some(v) if announced == Some(v) => Action::Nothing,
        Some(_) => Action::Announce,
    }
}

fn should_log(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= ERROR_LOG_GAP)
}

async fn fetch(app: &AppHandle) -> Result<Option<Update>, String> {
    app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())
}

/// Downloads and verifies first, then waits for any running switch or sync so the installer
/// (which exits the app on Windows) never interrupts a chat copy.
async fn install(app: &AppHandle, update: Update) -> Result<(), String> {
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    let _op = state.op_lock.lock().unwrap_or_else(|e| e.into_inner());
    append_journal(&state.backup_storage_dir, "update", "install", &format!("v{}", update.version), "ok");
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    let found = fetch(&app).await.map_err(check_failed)?;
    Ok(UpdateInfo {
        available: found.is_some(),
        version: found.as_ref().map(|u| u.version.clone()),
        notes: found.and_then(|u| u.body),
        current_version,
    })
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    match fetch(&app).await.map_err(check_failed)? {
        Some(u) => install(&app, u)
            .await
            .map_err(|e| t(format!("Güncelleme kurulamadı: {e}"), format!("Couldn't install the update: {e}"))),
        None => Err(t("Yeni sürüm bulunamadı.", "No new version found.").into()),
    }
}

fn check_failed(e: impl std::fmt::Display) -> String {
    t(format!("Güncelleme denetlenemedi: {e}"), format!("Couldn't check for updates: {e}"))
}

#[tauri::command(async)]
pub fn set_auto_update(enabled: bool, state: State<'_, AppState>) -> Result<FullDashboardPayload, String> {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.auto_update = enabled;
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    Ok(build_dashboard_state(&state))
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut announced: Option<String> = None;
        let mut last_error: Option<Instant> = None;
        let mut first = true;
        std::thread::sleep(FIRST_CHECK);
        loop {
            let enabled = state.config.lock().map(|c| c.automation.auto_update).unwrap_or(false);
            if enabled {
                let result = tauri::async_runtime::block_on(async {
                    let Some(u) = fetch(&app).await? else { return Ok(()) };
                    match decide(first, Some(&u.version), announced.as_deref()) {
                        Action::Install => install(&app, u).await,
                        Action::Announce => {
                            let _ = app
                                .notification()
                                .builder()
                                .title(t(format!("Yeni sürüm hazır: v{}", u.version), format!("New version ready: v{}", u.version)))
                                .body(t("Uygulamayı açıp \"Şimdi güncelle\"ye basabilirsin.", "Open the app and click \"Update now\"."))
                                .show();
                            let _ = app.emit("update-available", serde_json::json!({ "version": u.version }));
                            announced = Some(u.version);
                            Ok(())
                        }
                        Action::Nothing => Ok(()),
                    }
                });
                if let Err(e) = result {
                    if should_log(last_error, Instant::now()) {
                        last_error = Some(Instant::now());
                        append_journal(&state.backup_storage_dir, "update", "check", &e, "error");
                    }
                }
            }
            first = false;
            std::thread::sleep(INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_installs_later_announces_once() {
        assert_eq!(decide(true, None, None), Action::Nothing);
        assert_eq!(decide(true, Some("1.1.0"), None), Action::Install);
        assert_eq!(decide(false, Some("1.1.0"), None), Action::Announce);
        assert_eq!(decide(false, Some("1.1.0"), Some("1.1.0")), Action::Nothing);
        assert_eq!(decide(false, Some("1.2.0"), Some("1.1.0")), Action::Announce);
    }

    #[test]
    fn errors_logged_at_most_hourly() {
        let t = Instant::now();
        assert!(should_log(None, t));
        assert!(!should_log(Some(t), t + Duration::from_secs(59 * 60)));
        assert!(should_log(Some(t), t + ERROR_LOG_GAP));
    }
}
