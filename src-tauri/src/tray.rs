//! System tray: account switching, open window, quit.

use crate::accounts::{display_name, AccountProfile, UsageMetrics};
use crate::background::show_main_window;
use crate::{perform_switch, switch_plan, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

const TRAY_ID: &str = "main-tray";
const SWITCH_PREFIX: &str = "switch:";
const REFRESH_ID: &str = "refresh-usage";
static DOT_LEVEL: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Serialize)]
struct SwitchFinished {
    ok: bool,
    message: String,
}

fn build_menu(app: &AppHandle, accounts: &[AccountProfile]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for acc in accounts.iter().filter(|a| !a.is_active) {
        let id = format!("{SWITCH_PREFIX}{}", acc.account_uuid);
        menu.append(&MenuItem::with_id(app, id, switch_label(&display_name(acc), &acc.usage), true, None::<&str>)?)?;
    }
    if accounts.iter().any(|a| !a.is_active) {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(app, REFRESH_ID, "Limitleri yenile", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "show", "Claude Hesap Yöneticisini Aç", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?)?;
    Ok(menu)
}

fn switch_label(name: &str, usage: &UsageMetrics) -> String {
    match fullest(usage) {
        Some(p) => format!("{name} hesabına geç · %{p:.0}"),
        None => format!("{name} hesabına geç · veri yok"),
    }
}

/// Rebuilds the menu and tooltip from the current account list.
pub fn refresh(app: &AppHandle, accounts: &[AccountProfile]) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    if let Ok(menu) = build_menu(app, accounts) {
        let _ = tray.set_menu(Some(menu));
    }
    let active = accounts.iter().find(|a| a.is_active);
    let name = active.map(display_name).unwrap_or_else(|| "hesap yok".to_string());
    let usage = active.map(|a| &a.usage);
    let _ = tray.set_tooltip(Some(tooltip_text(&name, usage)));

    let level = usage.map_or(0, dot_level);
    // Repainting is only needed when the color changes (0 = no dot, the icon set in setup()).
    if DOT_LEVEL.swap(level, Ordering::Relaxed) != level {
        if let Some(base) = app.default_window_icon() {
            let _ = tray.set_icon(Some(with_dot(base, level)));
        }
    }
}

/// Fuller of the 5-hour and weekly windows, if any.
fn fullest(u: &UsageMetrics) -> Option<f64> {
    [&u.five_hour, &u.seven_day].into_iter().flatten().map(|w| w.utilization).reduce(f64::max)
}

/// 0 = no dot, 1 = green (<70), 2 = amber (70-89), 3 = red (>=90).
fn dot_level(u: &UsageMetrics) -> u8 {
    match fullest(u) {
        None => 0,
        Some(p) if p >= 90.0 => 3,
        Some(p) if p >= 70.0 => 2,
        Some(_) => 1,
    }
}

fn tooltip_text(name: &str, usage: Option<&UsageMetrics>) -> String {
    let mut s = format!("Claude: {name}");
    if let Some(u) = usage {
        if let Some(w) = &u.seven_day {
            s += &format!(" · haftalık %{:.0}", w.utilization);
        }
        if let Some(w) = &u.five_hour {
            s += &format!(" · 5 saatlik %{:.0}", w.utilization);
        }
    }
    s
}

/// Copy of `base` with a dark-outlined colored dot in the bottom-right corner (~40% of the icon).
fn with_dot(base: &Image<'_>, level: u8) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut px = base.rgba().to_vec();
    if level > 0 {
        let fill = match level {
            1 => [34, 197, 94, 255],
            2 => [245, 158, 11, 255],
            _ => [239, 68, 68, 255],
        };
        let r = w.min(h) as f32 * 0.2;
        let ring = (r / 4.0).max(1.0);
        let (cx, cy) = (w as f32 - r - 0.5, h as f32 - r - 0.5);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d <= r {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(if d <= r - ring { &fill } else { &[24, 24, 24, 255] });
                }
            }
        }
    }
    Image::new_owned(px, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::UsageWindow;

    fn usage(five: Option<f64>, week: Option<f64>) -> UsageMetrics {
        let w = |u| UsageWindow { utilization: u, resets_at: None };
        UsageMetrics { five_hour: five.map(w), seven_day: week.map(w), ..Default::default() }
    }

    #[test]
    fn dot_level_thresholds() {
        assert_eq!(dot_level(&usage(None, None)), 0);
        assert_eq!(dot_level(&usage(Some(10.0), Some(69.9))), 1);
        assert_eq!(dot_level(&usage(Some(10.0), Some(70.0))), 2);
        assert_eq!(dot_level(&usage(Some(89.0), Some(20.0))), 2);
        assert_eq!(dot_level(&usage(Some(95.0), Some(20.0))), 3);
    }

    #[test]
    fn switch_label_shows_fuller_window() {
        assert_eq!(switch_label("ornek", &usage(Some(5.0), Some(23.0))), "ornek hesabına geç · %23");
        assert_eq!(switch_label("ornek", &usage(None, None)), "ornek hesabına geç · veri yok");
    }

    #[test]
    fn tooltip_and_dot_pixels() {
        assert_eq!(tooltip_text("ornek", Some(&usage(None, Some(23.0)))), "Claude: ornek · haftalık %23");
        assert_eq!(tooltip_text("ornek", Some(&usage(Some(5.0), Some(23.0)))), "Claude: ornek · haftalık %23 · 5 saatlik %5");
        assert_eq!(tooltip_text("hesap yok", None), "Claude: hesap yok");
        let base = Image::new_owned(vec![0; 32 * 32 * 4], 32, 32);
        let img = with_dot(&base, 3);
        let i = (25 * 32 + 25) * 4;
        assert_eq!(&img.rgba()[i..i + 4], &[239, 68, 68, 255]);
        assert_eq!(&img.rgba()[0..4], &[0, 0, 0, 0]);
    }
}

pub fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&build_menu(app, &[])?)
        .tooltip("Claude Hesap & Oturum Yöneticisi")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            REFRESH_ID => {
                let app = app.clone();
                // Network calls: keep the tray event loop free. The command refreshes the tray itself.
                std::thread::spawn(move || {
                    let _ = crate::refresh_all_usage(app.clone(), app.state::<AppState>());
                });
            }
            id => {
                if let Some(uuid) = id.strip_prefix(SWITCH_PREFIX) {
                    let (app, uuid) = (app.clone(), uuid.to_string());
                    // The switch blocks for seconds; keep the tray event loop free.
                    std::thread::spawn(move || {
                        let state = app.state::<AppState>();
                        // Unapproved chats are missing in the target: let the window ask which ones to bring.
                        if switch_plan(&state, &uuid).ask {
                            show_main_window(&app);
                            let _ = app.emit("switch-choice-needed", serde_json::json!({ "targetAccountUuid": uuid }));
                            return;
                        }
                        let result = perform_switch(&app, &state, &uuid, Vec::new());
                        let payload = match result {
                            Ok(dash) => {
                                let name = dash.accounts.iter().find(|a| a.account_uuid.eq_ignore_ascii_case(&uuid)).map(display_name).unwrap_or_default();
                                SwitchFinished { ok: true, message: format!("{name} hesabına geçildi. Sohbetlerin hazır.") }
                            }
                            Err(e) => SwitchFinished { ok: false, message: e },
                        };
                        let _ = app.emit("switch-finished", payload);
                    });
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let _ = builder.build(app)?;
    Ok(())
}
