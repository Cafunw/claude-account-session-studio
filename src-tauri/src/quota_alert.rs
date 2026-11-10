//! Tells the user (once) when the active account's quota is nearly full. Never switches by itself:
//! a switch closes Claude Desktop, which could interrupt work.
use crate::accounts::{discover_accounts_in_root, display_name, AccountProfile, UsageMetrics};
use crate::paths::discover_real_roots;
use crate::AppState;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

const MAX_AGE_HOURS: i64 = 6;
const REARM_MARGIN: f64 = 10.0;

fn parse(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|d| d.with_timezone(&Utc))
}

/// Returns the window ("5 saatlik"/"haftalık") and its utilization to announce, updating `notified`.
/// A window is announced once per crossing; it re-arms below threshold-10 or after its reset time.
pub fn decide(
    notified: &mut HashSet<String>,
    uuid: &str,
    usage: &UsageMetrics,
    threshold: f64,
    now: DateTime<Utc>,
) -> Option<(&'static str, f64)> {
    let fresh = usage.fetched_at.as_deref().and_then(parse).is_some_and(|t| (now - t).num_hours() < MAX_AGE_HOURS);
    if !fresh {
        return None;
    }
    let mut out = None;
    for (name, w) in [("5 saatlik", &usage.five_hour), ("haftalık", &usage.seven_day)] {
        let Some(w) = w else { continue };
        let key = format!("{uuid}:{name}");
        let reset = w.resets_at.as_deref().and_then(parse).is_some_and(|r| r <= now);
        if reset || w.utilization < threshold - REARM_MARGIN {
            notified.remove(&key);
        } else if w.utilization >= threshold && out.is_none() && notified.insert(key) {
            out = Some((name, w.utilization));
        }
    }
    out
}

fn load(u: &UsageMetrics) -> f64 {
    let p = |w: &Option<crate::accounts::UsageWindow>| w.as_ref().map_or(0.0, |w| w.utilization);
    p(&u.five_hour).max(p(&u.seven_day))
}

pub fn message(active: &AccountProfile, window: &str, pct: f64, all: &[AccountProfile]) -> String {
    let best = all
        .iter()
        .filter(|a| a.account_uuid != active.account_uuid && (a.usage.five_hour.is_some() || a.usage.seven_day.is_some()))
        .min_by(|a, b| load(&a.usage).total_cmp(&load(&b.usage)));
    let tip = match best {
        Some(b) => format!("{} hesabına geçebilirsin (%{:.0} dolu).", display_name(b), load(&b.usage)),
        None => "Uygulamadan başka bir hesaba geçebilirsin.".to_string(),
    };
    format!("{} {} limiti %{:.0}. {}", display_name(active), window, pct, tip)
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut notified = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(180));
            let Some(root) = discover_real_roots().first().map(|r| PathBuf::from(&r.path)) else { continue };
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
            let (stored, threshold) = match state.config.lock() {
                Ok(c) => (c.accounts.clone(), c.automation.switch_threshold_percent),
                Err(_) => continue,
            };
            let mut accounts = discover_accounts_in_root(&root, Some(home.join(".claude.json").as_path()), &state.studio_dir, &stored);
            // The live API reading (refreshed by the UI) may be newer than Desktop's local history.
            for a in accounts.iter_mut() {
                if let Some(s) = stored.iter().find(|s| s.account_uuid == a.account_uuid) {
                    let newer = |x: &UsageMetrics, y: &UsageMetrics| x.fetched_at.as_deref().and_then(parse) > y.fetched_at.as_deref().and_then(parse);
                    if newer(&s.usage, &a.usage) {
                        a.usage = s.usage.clone();
                    }
                }
            }
            // Keeps the tray dot/tooltip fresh while the window is never opened.
            crate::tray::refresh(&app, &accounts);
            let Some(active) = accounts.iter().find(|a| a.is_active) else { continue };
            if let Some((window, pct)) = decide(&mut notified, &active.account_uuid, &active.usage, threshold, Utc::now()) {
                let _ = app
                    .notification()
                    .builder()
                    .title("Claude kotası dolmak üzere")
                    .body(message(active, window, pct, &accounts))
                    .show();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::UsageWindow;

    fn usage(five: f64, week: f64, now: DateTime<Utc>) -> UsageMetrics {
        let w = |u| Some(UsageWindow { utilization: u, resets_at: Some((now + chrono::Duration::hours(3)).to_rfc3339()) });
        UsageMetrics { five_hour: w(five), seven_day: w(week), fetched_at: Some(now.to_rfc3339()), ..Default::default() }
    }

    #[test]
    fn notifies_once_then_rearms() {
        let now = Utc::now();
        let mut n = HashSet::new();
        assert_eq!(decide(&mut n, "a", &usage(10.0, 92.0, now), 90.0, now), Some(("haftalık", 92.0)));
        assert_eq!(decide(&mut n, "a", &usage(10.0, 95.0, now), 90.0, now), None);
        assert_eq!(decide(&mut n, "a", &usage(10.0, 85.0, now), 90.0, now), None); // inside hysteresis
        assert_eq!(decide(&mut n, "a", &usage(10.0, 91.0, now), 90.0, now), None);
        assert_eq!(decide(&mut n, "a", &usage(10.0, 50.0, now), 90.0, now), None); // re-armed
        assert_eq!(decide(&mut n, "a", &usage(10.0, 93.0, now), 90.0, now), Some(("haftalık", 93.0)));
        // stale data is ignored
        let old = now - chrono::Duration::hours(7);
        assert_eq!(decide(&mut HashSet::new(), "b", &usage(99.0, 99.0, old), 90.0, now), None);
    }
}
