//! Quota history: the app's own samples in `<studio>/usage-history.json` merged (read-only) with
//! Claude Desktop's `plan-usage-history.json`, plus a plain linear forecast of the weekly limit.

use crate::accounts::AccountProfile;
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

const FILE: &str = "usage-history.json";
const MAX_AGE_MS: i64 = 60 * 24 * 3_600_000;
const MAX_SAMPLES: usize = 5000;
const MIN_GAP_MS: i64 = 15 * 60_000;
const H: f64 = 3_600_000.0;
/// A weekly % falling by more than this between two samples is a reset.
const RESET_DROP: f64 = 5.0;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSample {
    pub t: i64,
    pub account_uuid: String,
    pub fh: Option<f64>,
    pub sd: Option<f64>,
    #[serde(default)]
    pub src: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistory {
    pub samples: Vec<UsageSample>,
    /// Times of detected weekly resets inside the range.
    pub resets: Vec<i64>,
    pub forecast: String,
}

/// Read-modify-write of the history file; parallel dashboard builds would otherwise drop samples.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn load(studio_dir: &Path) -> Vec<UsageSample> {
    fs::read_to_string(studio_dir.join(FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn sample_of(acc: &AccountProfile, now_ms: i64) -> Option<UsageSample> {
    let u = &acc.usage;
    let (fh, sd) = (u.five_hour.as_ref().map(|w| w.utilization), u.seven_day.as_ref().map(|w| w.utilization));
    if fh.is_none() && sd.is_none() {
        return None;
    }
    // The reading's own time, so a Desktop record re-read every minute is one sample (and dedupes
    // against Desktop's file), not one per poll.
    let t = u
        .fetched_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or(now_ms, |d| d.timestamp_millis());
    Some(UsageSample { t, account_uuid: acc.account_uuid.clone(), fh, sd, src: u.source.clone() })
}

/// Appends `new` unless it repeats the account's last sample within 15 minutes. True if added.
fn push_if_new(store: &mut Vec<UsageSample>, new: UsageSample) -> bool {
    let last = store.iter().rev().find(|s| s.account_uuid.eq_ignore_ascii_case(&new.account_uuid));
    let keep = match last {
        None => true,
        Some(l) if new.t <= l.t => false,
        Some(l) => l.fh != new.fh || l.sd != new.sd || l.src != new.src || new.t - l.t >= MIN_GAP_MS,
    };
    if keep {
        store.push(new);
    }
    keep
}

fn prune(store: &mut Vec<UsageSample>, now_ms: i64) {
    store.retain(|s| s.t >= now_ms - MAX_AGE_MS);
    store.sort_by_key(|s| s.t);
    if store.len() > MAX_SAMPLES {
        store.drain(..store.len() - MAX_SAMPLES);
    }
}

/// Called after every usage (re)computation; writes only when a sample was added.
pub fn record(studio_dir: &Path, accounts: &[AccountProfile], now_ms: i64) {
    let _g = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(studio_dir);
    let mut added = false;
    for s in accounts.iter().filter_map(|a| sample_of(a, now_ms)) {
        added |= push_if_new(&mut store, s);
    }
    if added {
        prune(&mut store, now_ms);
        if let Ok(json) = serde_json::to_vec(&store) {
            let _ = crate::dpapi::atomic_write_verified(&studio_dir.join(FILE), &json);
        }
    }
}

/// Claude Desktop's samples for `org_uuid` (read-only): `{version, samples: [{t, org, u: {sd, fh}}]}`.
fn read_desktop_samples(data_root: &Path, org_uuid: &str, account_uuid: &str) -> Vec<UsageSample> {
    let Some(val) = fs::read_to_string(data_root.join("plan-usage-history.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    else {
        return Vec::new();
    };
    let Some(arr) = val.get("samples").unwrap_or(&val).as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter(|e| e.get("org").and_then(|v| v.as_str()).is_some_and(|o| o.eq_ignore_ascii_case(org_uuid)))
        .filter_map(|e| {
            let u = e.get("u")?;
            Some(UsageSample {
                t: e.get("t")?.as_i64()?,
                account_uuid: account_uuid.to_string(),
                fh: u.get("fh").and_then(|v| v.as_f64()),
                sd: u.get("sd").and_then(|v| v.as_f64()),
                src: "desktop-history".into(),
            })
        })
        .collect()
}

/// The account's samples from both sources, deduped by `t` (the app's own copy wins), oldest first.
fn merged_samples(studio_dir: &Path, data_root: &Path, acc: &AccountProfile) -> Vec<UsageSample> {
    let mut all: Vec<UsageSample> =
        load(studio_dir).into_iter().filter(|s| s.account_uuid.eq_ignore_ascii_case(&acc.account_uuid)).collect();
    if !acc.org_uuid.is_empty() {
        all.extend(read_desktop_samples(data_root, &acc.org_uuid, &acc.account_uuid));
    }
    all.sort_by_key(|s| s.t); // stable: the app's sample stays first among equal `t`
    all.dedup_by_key(|s| s.t);
    all
}

/// Times where the weekly % dropped sharply (a reset).
fn reset_times(samples: &[UsageSample]) -> Vec<i64> {
    let weekly: Vec<(i64, f64)> = samples.iter().filter_map(|s| Some((s.t, s.sd?))).collect();
    weekly.windows(2).filter(|w| w[1].1 + RESET_DROP < w[0].1).map(|w| w[1].0).collect()
}

#[derive(Debug, PartialEq)]
pub enum Forecast {
    NotEnoughData,
    AlreadyFull,
    /// Projected time (ms) the weekly limit fills; `reset_known` false means it may reset first.
    Fills { at: i64, reset_known: bool },
    WontFill { reset_known: bool },
}

/// Linear fit of the weekly % since the last reset, projected to 100%.
pub fn forecast(samples: &[UsageSample], now_ms: i64, resets_at_ms: Option<i64>) -> Forecast {
    let week = 7 * 24 * 3_600_000;
    let start = reset_times(samples)
        .last()
        .copied()
        .into_iter()
        .chain(resets_at_ms.map(|r| r - week))
        .max()
        .unwrap_or(i64::MIN);
    let pts: Vec<(f64, f64)> =
        samples.iter().filter(|s| s.t >= start).filter_map(|s| Some((s.t as f64, s.sd?))).collect();
    if pts.last().is_some_and(|p| p.1 >= 100.0) {
        return Forecast::AlreadyFull;
    }
    let span = pts.last().map_or(0.0, |l| l.0 - pts[0].0);
    if pts.len() < 3 || span < 6.0 * H {
        return Forecast::NotEnoughData;
    }
    let n = pts.len() as f64;
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let slope = sxy / sxx; // % per ms
    // Without a known reset the window still ends within a week of now.
    let deadline = resets_at_ms.unwrap_or(now_ms + week);
    let reset_known = resets_at_ms.is_some();
    if slope <= 0.0 {
        return Forecast::WontFill { reset_known };
    }
    let at = ((mx + (100.0 - my) / slope) as i64).max(now_ms);
    if at < deadline {
        Forecast::Fills { at, reset_known }
    } else {
        Forecast::WontFill { reset_known }
    }
}

pub fn forecast_text(f: &Forecast, tz: &FixedOffset) -> String {
    const DAYS: [&str; 7] = ["pazartesi", "salı", "çarşamba", "perşembe", "cuma", "cumartesi", "pazar"];
    let unknown = "; sıfırlanma zamanı bilinmiyor";
    match f {
        Forecast::NotEnoughData => "Tahmin için yeterli veri yok".into(),
        Forecast::AlreadyFull => "Haftalık limit dolmuş".into(),
        Forecast::Fills { at, reset_known } => {
            let d = DateTime::<Utc>::from_timestamp_millis(*at).unwrap_or_default().with_timezone(tz);
            let day = DAYS[chrono::Datelike::weekday(&d).num_days_from_monday() as usize];
            format!("Bu hızla {day} {} civarı dolar{}", d.format("%H:%M"), if *reset_known { "" } else { unknown })
        }
        Forecast::WontFill { reset_known: true } => "Bu hızla sıfırlanmadan önce dolmaz".into(),
        Forecast::WontFill { reset_known: false } => format!("Bu hızla bir hafta içinde dolmaz{unknown}"),
    }
}

pub fn history_for(studio_dir: &Path, data_root: &Path, acc: &AccountProfile, days: u32, now_ms: i64, tz: &FixedOffset) -> UsageHistory {
    let all = merged_samples(studio_dir, data_root, acc);
    let resets_at = acc
        .usage
        .seven_day
        .as_ref()
        .and_then(|w| w.resets_at.as_deref())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis());
    let forecast = forecast_text(&forecast(&all, now_ms, resets_at), tz);
    let from = now_ms - i64::from(days.clamp(1, 60)) * 24 * 3_600_000;
    let resets = reset_times(&all).into_iter().filter(|&t| t >= from).collect();
    UsageHistory { samples: all.into_iter().filter(|s| s.t >= from).collect(), resets, forecast }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{UsageMetrics, UsageWindow};

    fn s(t_h: f64, sd: f64) -> UsageSample {
        UsageSample { t: (t_h * H) as i64, account_uuid: "acc-a".into(), fh: None, sd: Some(sd), src: "oauth-api".into() }
    }

    fn account(uuid: &str, org: &str) -> AccountProfile {
        serde_json::from_value(serde_json::json!({
            "id": uuid, "label": "", "email": "x@example.com", "accountUuid": uuid, "orgUuid": org,
            "planType": "", "rateLimitTier": "", "hasDpapiCredentials": false, "isActive": false,
            "sessionCount": 0, "healthySessionCount": 0, "lastActivityAt": null,
            "usage": UsageMetrics::default(), "addedAt": ""
        }))
        .unwrap()
    }

    #[test]
    fn push_skips_repeats_within_15_minutes() {
        let mut v = vec![s(0.0, 10.0)];
        assert!(!push_if_new(&mut v, s(0.0, 10.0)), "same reading");
        assert!(!push_if_new(&mut v, s(0.1, 10.0)), "unchanged, 6 min later");
        assert!(push_if_new(&mut v, s(0.1, 11.0)), "value changed");
        let mut other_src = s(0.2, 11.0);
        other_src.src = "desktop-history".into();
        assert!(push_if_new(&mut v, other_src), "source changed");
        assert!(push_if_new(&mut v, s(0.5, 11.0)), "15+ min passed");
        let mut b = s(0.5, 11.0);
        b.account_uuid = "acc-b".into();
        assert!(push_if_new(&mut v, b), "other account has its own last sample");
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn prune_drops_old_and_caps_count() {
        let now = (70.0 * 24.0 * H) as i64;
        let mut v: Vec<_> = (0..6000).map(|i| s(70.0 * 24.0 - i as f64 * 0.1, 1.0)).collect();
        v.push(s(1.0, 1.0)); // 69 days old
        prune(&mut v, now);
        assert_eq!(v.len(), MAX_SAMPLES);
        assert!(v.windows(2).all(|w| w[0].t <= w[1].t));
        assert_eq!(v.last().unwrap().t, now, "newest kept");
    }

    #[test]
    fn record_writes_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut acc = account("acc-a", "org-a");
        acc.usage.seven_day = Some(UsageWindow { utilization: 40.0, resets_at: None });
        acc.usage.fetched_at = Some("2026-01-01T00:00:00Z".into());
        record(dir.path(), &[acc.clone(), account("acc-none", "org-x")], 0);
        record(dir.path(), &[acc], 0); // same reading again: no new sample
        let v = load(dir.path());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].sd, Some(40.0));
    }

    #[test]
    fn merges_with_desktop_samples_dedupes_and_sorts() {
        let studio = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let own = vec![s(1.0, 10.0), s(3.0, 30.0), { let mut x = s(2.0, 99.0); x.account_uuid = "acc-b".into(); x }];
        fs::write(studio.path().join(FILE), serde_json::to_vec(&own).unwrap()).unwrap();
        let (t1, t2, t3) = ((1.0 * H) as i64, (2.0 * H) as i64, (4.0 * H) as i64);
        fs::write(
            root.path().join("plan-usage-history.json"),
            format!(
                r#"{{"version":2,"samples":[
                {{"t":{t3},"org":"ORG-A","u":{{"sd":40,"fh":5}}}},
                {{"t":{t1},"org":"org-a","u":{{"sd":11}}}},
                {{"t":{t2},"org":"org-a","u":{{"sd":20}}}},
                {{"t":{t2},"org":"org-other","u":{{"sd":77}}}}]}}"#
            ),
        )
        .unwrap();
        let v = merged_samples(studio.path(), root.path(), &account("ACC-A", "org-a"));
        let got: Vec<_> = v.iter().map(|x| (x.t, x.sd.unwrap())).collect();
        assert_eq!(got, vec![(t1, 10.0), (t2, 20.0), ((3.0 * H) as i64, 30.0), (t3, 40.0)], "app's copy wins at t1");
        assert_eq!(v[3].fh, Some(5.0));
        // No Desktop file: only the app's samples.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(merged_samples(studio.path(), empty.path(), &account("acc-a", "org-a")).len(), 2);
    }

    #[test]
    fn forecast_cases() {
        let now = (12.0 * H) as i64;
        let reset = Some((48.0 * H) as i64);
        // 5%/h from 10% at t=0 reaches 100% at 18 h, before the reset at 48 h.
        let rising: Vec<_> = (0..=12).map(|h| s(h as f64, 10.0 + 5.0 * h as f64)).collect();
        match forecast(&rising, now, reset) {
            Forecast::Fills { at, reset_known: true } => assert!((at - (18.0 * H) as i64).abs() < 1000),
            f => panic!("{f:?}"),
        }
        // Slow: 0.5%/h won't fill before the reset.
        let slow: Vec<_> = (0..=12).map(|h| s(h as f64, 10.0 + 0.5 * h as f64)).collect();
        assert_eq!(forecast(&slow, now, reset), Forecast::WontFill { reset_known: true });
        // Too few points or too short a span.
        assert_eq!(forecast(&rising[..2], now, reset), Forecast::NotEnoughData);
        let short: Vec<_> = (0..5).map(|i| s(i as f64, 10.0 + i as f64)).collect();
        assert_eq!(forecast(&short, now, reset), Forecast::NotEnoughData);
        // Only samples after the sharp drop count: before it the trend was steep.
        let mut sawtooth: Vec<_> = (0..=10).map(|h| s(h as f64 - 20.0, 9.0 * h as f64)).collect();
        sawtooth.extend((0..=12).map(|h| s(h as f64, 0.5 * h as f64)));
        assert_eq!(forecast(&sawtooth, now, reset), Forecast::WontFill { reset_known: true });
        // Unknown reset: still projects, capped at a week.
        assert!(matches!(forecast(&rising, now, None), Forecast::Fills { reset_known: false, .. }));
        assert_eq!(forecast(&slow, now, None), Forecast::WontFill { reset_known: false });
        assert_eq!(forecast(&[s(0.0, 50.0), s(3.0, 80.0), s(7.0, 100.0)], now, reset), Forecast::AlreadyFull);
    }

    #[test]
    fn forecast_text_names_day_and_time() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        // 2026-11-12 is a Thursday; 15:30 UTC is 18:30 at +03:00.
        let at = DateTime::parse_from_rfc3339("2026-11-12T15:30:00Z").unwrap().timestamp_millis();
        assert_eq!(forecast_text(&Forecast::Fills { at, reset_known: true }, &tz), "Bu hızla perşembe 18:30 civarı dolar");
        assert!(forecast_text(&Forecast::Fills { at, reset_known: false }, &tz).ends_with("sıfırlanma zamanı bilinmiyor"));
        assert_eq!(forecast_text(&Forecast::WontFill { reset_known: true }, &tz), "Bu hızla sıfırlanmadan önce dolmaz");
        assert_eq!(forecast_text(&Forecast::NotEnoughData, &tz), "Tahmin için yeterli veri yok");
    }
}
