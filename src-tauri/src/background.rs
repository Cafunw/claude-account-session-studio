//! Arka planda çalışma: pencere kapanınca tepsiye gizlenme, tek örnek ve eski Windows başlangıç kaydının temizlenmesi.
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Pencere hemen görünmez (tauri.conf.json); her açılışta burada gösterilir.
pub fn show_window(window: &WebviewWindow) {
    let _ = window.show();
}

/// Windows ile başlatma özelliği kaldırıldı; eski sürümlerin yazdığı Run kaydını siler.
pub fn remove_legacy_autostart(app: &AppHandle) {
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) {
        let _ = al.disable();
    }
}
