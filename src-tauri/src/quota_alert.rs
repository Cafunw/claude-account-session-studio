//! Tells the user (once) when the active account's quota is nearly full. Never switches by itself:
//! a switch closes Claude Desktop, which could interrupt work.
use crate::accounts::{discover_accounts_in_root, display_name, AccountProfile, UsageMetrics};
use crate::i18n::t;
use crate::paths::discover_real_roots;
use crate::AppState;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

const MAX_AGE_HOURS: i64 = 6;
const REARM_MARGIN: f64 = 10.0;
/// A suggested account must be at least this many points emptier than the active one.
const MIN_GAP: f64 = 20.0;

fn parse(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|d| d.with_timezone(&Utc))
}

fn is_fresh(usage: &UsageMetrics, now: DateTime<Utc>) -> bool {
    usage.fetched_at.as_deref().and_then(parse).is_some_and(|t| (now - t).num_hours() < MAX_AGE_HOURS)
}

/// Returns the window key ("5 saatlik"/"haftalık", see `window_label`) and its utilization to announce, updating `notified`.
/// A window is announced once per crossing; it re-arms below threshold-10 or after its reset time.
pub fn decide(
    notified: &mut HashSet<String>,
    uuid: &str,
    usage: &UsageMetrics,
    threshold: f64,
    now: DateTime<Utc>,
) -> Option<(&'static str, f64)> {
    if !is_fresh(usage, now) {
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

const RESET_HIGH: f64 = 50.0;
const RESET_DROP: f64 = 40.0;

/// Last seen (utilization, resets_at) per "<uuid>:<window>".
pub type Seen = HashMap<String, (f64, Option<DateTime<Utc>>)>;

/// True when a window that was high is now clearly lower: a sharp drop, or its known reset time has
/// passed and the reading went down (a stale reading that still shows the old value does not count).
fn has_reset(prev: (f64, Option<DateTime<Utc>>), cur: f64, now: DateTime<Utc>) -> bool {
    let (was, resets_at) = prev;
    was >= RESET_HIGH && (cur <= was - RESET_DROP || (cur < was && resets_at.is_some_and(|r| r <= now)))
}

/// Windows of this account that just reset, updating `seen`. The first observation never reports.
pub fn observe_resets(seen: &mut Seen, uuid: &str, usage: &UsageMetrics, now: DateTime<Utc>) -> Vec<&'static str> {
    if !is_fresh(usage, now) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, w) in [("5 saatlik", &usage.five_hour), ("haftalık", &usage.seven_day)] {
        let Some(w) = w else { continue };
        let reset_at = w.resets_at.as_deref().and_then(parse);
        if let Some(prev) = seen.insert(format!("{uuid}:{name}"), (w.utilization, reset_at)) {
            if has_reset(prev, w.utilization, now) {
                out.push(name);
            }
        }
    }
    out
}

/// The window keys above stay fixed (they key `notified`/`seen`); this is their wording.
fn window_label(key: &str) -> &'static str {
    match key {
        "5 saatlik" => t("5 saatlik", "5-hour"),
        _ => t("haftalık", "weekly"),
    }
}

fn load(u: &UsageMetrics) -> f64 {
    let p = |w: &Option<crate::accounts::UsageWindow>| w.as_ref().map_or(0.0, |w| w.utilization);
    p(&u.five_hour).max(p(&u.seven_day))
}

/// The emptiest other account with fresh data that is under the threshold and at least `MIN_GAP`
/// points emptier than the active one; None when nothing is worth switching to.
pub fn suggest<'a>(active: &AccountProfile, all: &'a [AccountProfile], threshold: f64, now: DateTime<Utc>) -> Option<&'a AccountProfile> {
    let ceiling = load(&active.usage) - MIN_GAP;
    all.iter()
        .filter(|a| a.account_uuid != active.account_uuid && is_fresh(&a.usage, now) && load(&a.usage) < threshold && load(&a.usage) <= ceiling)
        .min_by(|a, b| load(&a.usage).total_cmp(&load(&b.usage)))
}

pub fn message(active: &AccountProfile, window: &str, pct: f64, suggestion: Option<&AccountProfile>) -> String {
    let tip = match suggestion {
        Some(b) => t(
            format!("{} hesabına geçebilirsin (%{:.0} dolu).", display_name(b), load(&b.usage)),
            format!("You can switch to {} ({:.0}% used).", display_name(b), load(&b.usage)),
        ),
        None => t("Uygulamadan başka bir hesaba geçebilirsin.", "You can switch to another account in the app.").to_string(),
    };
    let window = window_label(window);
    t(
        format!("{} {} limiti %{:.0}. {}", display_name(active), window, pct, tip),
        format!("{}: {} limit at {:.0}%. {}", display_name(active), window, pct, tip),
    )
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut notified = HashSet::new();
        let mut seen = Seen::new();
        loop {
            std::thread::sleep(Duration::from_secs(180));
            let Some(root) = discover_real_roots().first().map(|r| PathBuf::from(&r.path)) else { continue };
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
            let (stored, threshold, notify_reset) = match state.config.lock() {
                Ok(c) => (c.accounts.clone(), c.automation.switch_threshold_percent, c.automation.notify_on_reset),
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
            let now = Utc::now();
            for a in &accounts {
                // Always observed (keeps the baseline current), announced only when enabled.
                for window in observe_resets(&mut seen, &a.account_uuid, &a.usage, now) {
                    if notify_reset {
                        let _ = app
                            .notification()
                            .builder()
                            .title(t("Claude limiti yenilendi", "Claude limit reset"))
                            .body(t(
                                format!("{} hesabının {} limiti yenilendi", display_name(a), window_label(window)),
                                format!("The {} limit of {} has reset", window_label(window), display_name(a)),
                            ))
                            .show();
                    }
                }
            }
            let Some(active) = accounts.iter().find(|a| a.is_active) else { continue };
            if let Some((window, pct)) = decide(&mut notified, &active.account_uuid, &active.usage, threshold, Utc::now()) {
                let pick = suggest(active, &accounts, threshold, Utc::now());
                let _ = app
                    .notification()
                    .builder()
                    .title(t("Claude kotası dolmak üzere", "Claude quota almost full"))
                    .body(message(active, window, pct, pick))
                    .show();
                // The in-app banner; the app still never switches by itself.
                if let Some(to) = pick {
                    let _ = app.emit(
                        "switch-suggested",
                        serde_json::json!({
                            "fromUuid": active.account_uuid,
                            "toUuid": to.account_uuid,
                            "window": window_label(window),
                            "pct": pct,
                            "toPct": load(&to.usage),
                        }),
                    );
                }
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
    fn reset_rule() {
        let now = Utc::now();
        let mut s = Seen::new();
        let at = |u: f64, resets: i64| {
            let w = |u| Some(UsageWindow { utilization: u, resets_at: Some((now + chrono::Duration::minutes(resets)).to_rfc3339()) });
            UsageMetrics { five_hour: w(u), fetched_at: Some(now.to_rfc3339()), ..Default::default() }
        };
        // first observation: never announced, even if it looks like a reset
        assert!(observe_resets(&mut s, "a", &at(80.0, 60), now).is_empty());
        // sharp drop from a high value
        assert_eq!(observe_resets(&mut s, "a", &at(5.0, 300), now), vec!["5 saatlik"]);
        // already low: no repeat
        assert!(observe_resets(&mut s, "a", &at(3.0, 300), now).is_empty());
        // small drop, reset time still ahead: not a reset
        observe_resets(&mut s, "b", &at(90.0, 60), now);
        assert!(observe_resets(&mut s, "b", &at(70.0, 60), now).is_empty());
        // reset time passed and the reading went down
        observe_resets(&mut s, "c", &at(60.0, -1), now);
        assert_eq!(observe_resets(&mut s, "c", &at(45.0, 300), now), vec!["5 saatlik"]);
        // reset time passed but the reading is stale (unchanged): wait for fresh data
        observe_resets(&mut s, "d", &at(60.0, -1), now);
        assert!(observe_resets(&mut s, "d", &at(60.0, -1), now).is_empty());
        // below the "was high" floor
        observe_resets(&mut s, "e", &at(40.0, 60), now);
        assert!(observe_resets(&mut s, "e", &at(0.0, 300), now).is_empty());
        // stale fetch is ignored
        let mut old = at(0.0, 300);
        old.fetched_at = Some((now - chrono::Duration::hours(7)).to_rfc3339());
        observe_resets(&mut s, "f", &at(90.0, 60), now);
        assert!(observe_resets(&mut s, "f", &old, now).is_empty());
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

    #[test]
    fn message_in_both_languages() {
        let acc = |uuid: &str, email: &str, week: f64| {
            let mut a: AccountProfile = serde_json::from_value(serde_json::json!({
                "id": "", "label": "", "email": email, "accountUuid": uuid, "orgUuid": "", "planType": "",
                "rateLimitTier": "", "hasDpapiCredentials": false, "isActive": false, "sessionCount": 0,
                "healthySessionCount": 0, "lastActivityAt": null, "usage": {"source": ""}, "addedAt": ""
            }))
            .unwrap();
            a.usage = usage(10.0, week, Utc::now());
            a
        };
        let all = [acc("a", "a@x.com", 92.0), acc("b", "b@x.com", 30.0)];
        let pick = suggest(&all[0], &all, 90.0, Utc::now());
        crate::i18n::set_test_lang(crate::i18n::Lang::Tr);
        assert_eq!(message(&all[0], "haftalık", 92.0, pick), "a@x.com haftalık limiti %92. b@x.com hesabına geçebilirsin (%30 dolu).");
        assert_eq!(message(&all[0], "haftalık", 92.0, None), "a@x.com haftalık limiti %92. Uygulamadan başka bir hesaba geçebilirsin.");
        crate::i18n::set_test_lang(crate::i18n::Lang::En);
        assert_eq!(message(&all[0], "haftalık", 92.0, pick), "a@x.com: weekly limit at 92%. You can switch to b@x.com (30% used).");
    }

    #[test]
    fn suggest_rules() {
        let now = Utc::now();
        let acc = |uuid: &str, u: UsageMetrics| {
            let mut a: AccountProfile = serde_json::from_value(serde_json::json!({
                "id": "", "label": "", "email": format!("{uuid}@x.com"), "accountUuid": uuid, "orgUuid": "", "planType": "",
                "rateLimitTier": "", "hasDpapiCredentials": false, "isActive": false, "sessionCount": 0,
                "healthySessionCount": 0, "lastActivityAt": null, "usage": {"source": ""}, "addedAt": ""
            }))
            .unwrap();
            a.usage = u;
            a
        };
        let active = acc("a", usage(10.0, 92.0, now));
        let pick = |others: Vec<AccountProfile>| {
            let mut all = vec![active.clone()];
            all.extend(others);
            suggest(&active, &all, 90.0, now).map(|a| a.account_uuid.clone())
        };
        // the emptiest of the qualifying ones wins
        assert_eq!(pick(vec![acc("b", usage(10.0, 60.0, now)), acc("c", usage(40.0, 20.0, now))]), Some("c".into()));
        // not 20 points emptier than the active one (72 -> 71 is too close)
        assert_eq!(pick(vec![acc("b", usage(10.0, 73.0, now))]), None);
        assert_eq!(pick(vec![acc("b", usage(10.0, 72.0, now))]), Some("b".into()));
        // at or above the threshold is never suggested, even with a big gap to a 100% active account
        let full = acc("a", usage(10.0, 100.0, now));
        let all = [full.clone(), acc("b", usage(10.0, 91.0, now))];
        assert!(suggest(&full, &all, 90.0, now).is_none());
        // stale or missing data does not count
        let old = now - chrono::Duration::hours(7);
        assert_eq!(pick(vec![acc("b", usage(1.0, 1.0, old))]), None);
        assert_eq!(pick(vec![acc("b", UsageMetrics::default())]), None);
        // the active account itself is skipped; no others: nothing
        assert_eq!(pick(vec![]), None);
    }
}
