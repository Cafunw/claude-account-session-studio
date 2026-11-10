//! Optional global shortcut that switches to the next account (same flow as the tray menu).
use crate::accounts::save_studio_config;
use crate::{build_dashboard_state, tray, AppState, FullDashboardPayload};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::TryLockError;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// The choices offered in Ayarlar; anything else is refused.
const ALLOWED: [&str; 3] = ["Ctrl+Alt+K", "Ctrl+Alt+J", "Ctrl+Shift+F12"];
const TAKEN: &str = "Bu kısayol başka bir uygulamada kullanılıyor";

/// Covers the whole press (finding the target + the switch), so a second press never queues behind it.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The item after the active one, wrapping around; `None` when nothing is active or it is the only one.
fn next_after_active<T>(items: &[T], is_active: impl Fn(&T) -> bool) -> Option<&T> {
    let i = items.iter().position(is_active)?;
    items.get((i + 1) % items.len()).filter(|_| items.len() > 1)
}

fn on_press(app: &AppHandle) {
    let state = app.state::<AppState>();
    // A switch started elsewhere (window, tray) holds op_lock; the guard is dropped right here.
    if matches!(state.op_lock.try_lock(), Err(TryLockError::WouldBlock)) {
        return;
    }
    // The dashboard's account list already follows the stored order.
    let accounts = build_dashboard_state(&state).accounts;
    if let Some(next) = next_after_active(&accounts, |a| a.is_active) {
        tray::switch_to(app, &next.account_uuid);
    }
}

/// Replaces whatever is registered; "" leaves nothing registered.
fn register(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let keys = app.global_shortcut();
    keys.unregister_all().map_err(|e| e.to_string())?;
    if shortcut.is_empty() {
        return Ok(());
    }
    keys.on_shortcut(shortcut, |app, _, event| {
        if event.state() != ShortcutState::Pressed || RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            on_press(&app);
            RUNNING.store(false, Ordering::SeqCst);
        });
    })
    .map_err(|_| TAKEN.to_string())
}

/// Startup: registers the saved shortcut; a failure (taken meanwhile) just leaves it unregistered.
pub fn apply_saved(app: &AppHandle) {
    let saved = app.state::<AppState>().config.lock().map(|c| c.automation.switch_shortcut.clone()).unwrap_or_default();
    let _ = register(app, &saved);
}

#[tauri::command(async)]
pub fn set_switch_shortcut(
    shortcut: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FullDashboardPayload, String> {
    if !shortcut.is_empty() && !ALLOWED.contains(&shortcut.as_str()) {
        return Err(format!("Desteklenmeyen kısayol: {shortcut}"));
    }
    let old = state.config.lock().unwrap().automation.switch_shortcut.clone();
    if let Err(e) = register(&app, &shortcut) {
        let _ = register(&app, &old); // keep the previous shortcut working
        return Err(e);
    }
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.automation.switch_shortcut = shortcut;
        save_studio_config(&state.studio_dir, &cfg)?;
    }
    Ok(build_dashboard_state(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_account_wraps_and_needs_an_active_one() {
        let accs = [("a", false), ("b", true), ("c", false)];
        let next = |v: &[(&'static str, bool)]| next_after_active(v, |x| x.1).map(|x| x.0);
        assert_eq!(next(&accs), Some("c"));
        assert_eq!(next(&[("a", false), ("b", false), ("c", true)]), Some("a"));
        assert_eq!(next(&[("a", true)]), None, "a single account has no next");
        assert_eq!(next(&[("a", false), ("b", false)]), None, "no active account");
        assert_eq!(next(&[]), None);
    }
}
