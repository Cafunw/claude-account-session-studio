//! Stops every write when Claude Desktop's session pointer files or sidebar config no longer look
//! like what `sync_engine` understands (e.g. after a Desktop update). `check` is read-only.
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use crate::i18n::t;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// Prefix of the error `execute_sync` returns when the guard trips (callers match on it), Turkish and English.
const STOPPED_TR: &str =
    "Claude Desktop'un dosya yapısı değişmiş görünüyor; sohbetlere dokunmamak için aktarım durduruldu.";
const STOPPED_EN: &str = "Claude Desktop's file layout seems to have changed; copying is stopped so your chats stay untouched.";

pub fn stopped() -> &'static str {
    t(STOPPED_TR, STOPPED_EN)
}

/// Either language: the language may change between the error and the check.
pub fn is_guard_error(e: &str) -> bool {
    e.starts_with(STOPPED_TR) || e.starts_with(STOPPED_EN)
}

// Thresholds: a few broken files are normal (Desktop mid-write, old crashes) and sync already skips
// them, so with fewer than MIN_FILES pointers we never call it a format change. With enough files it
// trips when at least half fail, or when one key fails in every one of the NEWEST (by mtime) files:
// Desktop rewrites only the chats it touches, so a new format shows up first in the newest files.
const MIN_FILES: usize = 3;
const NEWEST: usize = 5;

/// `local_*.json` keys `sync_engine` relies on, with the type it reads them as.
const REQUIRED: [(&str, Kind); 4] = [
    ("sessionId", Kind::LocalId),
    ("cliSessionId", Kind::StrOrNull),
    ("createdAt", Kind::Int),
    ("lastActivityAt", Kind::Int),
];
/// Checked only when present and not null. `title` is here: Desktop writes it after a chat's first turn.
const OPTIONAL: [(&str, Kind); 6] = [
    ("title", Kind::Str),
    ("cwd", Kind::Str),
    ("originCwd", Kind::Str),
    ("completedTurns", Kind::Int),
    ("bridgeSessionIds", Kind::Array),
    ("spawnedFrom", Kind::Object),
];

#[derive(Clone, Copy)]
enum Kind {
    LocalId,
    Str,
    StrOrNull,
    Int,
    Array,
    Object,
}

fn fits(v: &Value, k: Kind) -> bool {
    match k {
        Kind::LocalId => v.as_str().is_some_and(|s| s.starts_with("local_")),
        Kind::Str => v.is_string(),
        Kind::StrOrNull => v.is_string() || v.is_null(),
        Kind::Int => v.as_i64().is_some(),
        Kind::Array => v.is_array(),
        Kind::Object => v.is_object(),
    }
}

/// Keys that are missing or have the wrong type; every required key when the file is not an object.
fn bad_keys(raw: &str) -> Vec<&'static str> {
    let Some(obj) = serde_json::from_str::<Value>(raw).ok().and_then(|v| v.as_object().cloned()) else {
        return REQUIRED.iter().map(|(k, _)| *k).collect();
    };
    let req = REQUIRED.iter().filter(|(k, t)| !obj.get(*k).is_some_and(|v| fits(v, *t)));
    let opt = OPTIONAL.iter().filter(|(k, t)| obj.get(*k).is_some_and(|v| !v.is_null() && !fits(v, *t)));
    req.chain(opt).map(|(k, _)| *k).collect()
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices()
            .all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

fn is_pointer(p: &Path) -> bool {
    p.is_file()
        && p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("local_") && n.ends_with(".json"))
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// `None` when everything looks as expected, else a short Turkish description of what changed.
pub fn check(data_root: &Path) -> Option<String> {
    check_sessions(&data_root.join("claude-code-sessions")).or_else(|| check_config(data_root))
}

fn check_sessions(root: &Path) -> Option<String> {
    // Expected: claude-code-sessions/<account uuid>/<org>/local_*.json. The org name is not checked:
    // only the depth decides where sync writes.
    let (mut pointers, mut stray) = (Vec::new(), 0usize);
    for acc in entries(root) {
        if is_pointer(&acc) {
            stray += 1;
            continue;
        }
        let acc_ok = acc.file_name().and_then(|n| n.to_str()).is_some_and(is_uuid);
        for org in entries(&acc) {
            if is_pointer(&org) {
                stray += 1;
                continue;
            }
            for f in entries(&org) {
                if is_pointer(&f) && acc_ok {
                    pointers.push(f);
                } else if is_pointer(&f) || entries(&f).iter().any(|p| is_pointer(p)) {
                    stray += 1;
                }
            }
        }
    }
    if pointers.is_empty() {
        return (stray > 0).then(|| {
            t(
                "Sohbet dosyaları beklenen klasör düzeninde değil (claude-code-sessions/<hesap>/<kuruluş>/local_*.json).",
                "Chat files are not in the expected folder layout (claude-code-sessions/<account>/<org>/local_*.json).",
            )
            .to_string()
        });
    }

    // Unreadable (locked) files are skipped, not counted.
    let mut files: Vec<(SystemTime, String, Vec<&'static str>)> = pointers
        .iter()
        .filter_map(|p| {
            let raw = fs::read_to_string(p).ok()?;
            let mtime = fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let name = p.file_name()?.to_string_lossy().into_owned();
            Some((mtime, name, bad_keys(&raw)))
        })
        .collect();
    if files.len() < MIN_FILES {
        return None;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));

    let newest = &files[..files.len().min(NEWEST)];
    let mut common: HashSet<&str> = newest[0].2.iter().copied().collect();
    for f in &newest[1..] {
        common.retain(|k| f.2.contains(k));
    }
    if !common.is_empty() {
        let mut keys: Vec<&str> = common.into_iter().collect();
        keys.sort();
        let (n, keys) = (newest.len(), keys.join(", "));
        return Some(t(
            format!("En yeni {n} sohbet dosyasının hiçbirinde beklenen alan yok ya da türü değişmiş: {keys}."),
            format!("None of the {n} newest chat files has the expected fields, or their type changed: {keys}."),
        ));
    }

    let bad: Vec<_> = files.iter().filter(|f| !f.2.is_empty()).collect();
    (bad.len() >= MIN_FILES && bad.len() * 2 >= files.len()).then(|| {
        let (b, n, name, keys) = (bad.len(), files.len(), &bad[0].1, bad[0].2.join(", "));
        t(
            format!("{b}/{n} sohbet dosyası beklenen biçimde değil (ör. {name}: {keys})."),
            format!("{b}/{n} chat files are not in the expected format (e.g. {name}: {keys})."),
        )
    })
}

fn check_config(data_root: &Path) -> Option<String> {
    let raw = fs::read_to_string(data_root.join("claude_desktop_config.json")).ok()?;
    // Unparseable usually means Desktop is mid-write; the sidebar merge refuses such a file anyway.
    let cfg: Value = serde_json::from_str(&raw).ok()?;
    if !cfg.is_object() {
        return Some(t("claude_desktop_config.json artık bir JSON nesnesi değil.", "claude_desktop_config.json is no longer a JSON object.").to_string());
    }
    let scopes = cfg.pointer("/preferences/epitaxyPrefs/dframe-group-scopes")?;
    let scope_ok = |s: &Value| {
        let groups_ok = s.get("groups").map_or(true, |g| {
            g.as_array().is_some_and(|a| a.iter().all(|x| x.get("id").is_some_and(Value::is_string)))
        });
        let assign_ok = s.get("assignments").map_or(true, |a| {
            a.as_object().is_some_and(|m| m.values().all(|v| v.is_string() || v.is_null()))
        });
        s.is_object() && groups_ok && assign_ok
    };
    let ok = scopes.as_object().is_some_and(|m| m.values().all(scope_ok));
    (!ok).then(|| {
        t(
            "claude_desktop_config.json içindeki kenar çubuğu bölümleri (dframe-group-scopes) beklenen biçimde değil.",
            "The sidebar sections (dframe-group-scopes) in claude_desktop_config.json are not in the expected format.",
        )
        .to_string()
    })
}

/// One Windows notification when the guard starts tripping; re-arms once the format looks fine again.
pub fn spawn_alert(app: AppHandle) {
    std::thread::spawn(move || {
        let mut shown = false;
        loop {
            if let Some(root) = crate::paths::discover_real_roots().first().map(|r| PathBuf::from(&r.path)) {
                match check(&root) {
                    Some(detail) if !shown => {
                        shown = true;
                        let _ = app.notification().builder().title(t("Sohbet aktarımı durduruldu", "Chat copying stopped")).body(detail).show();
                    }
                    Some(_) => {}
                    None => shown = false,
                }
            }
            std::thread::sleep(Duration::from_secs(180));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ACC: &str = "aaaa1111-0000-0000-0000-000000000001";
    const ORG: &str = "bbbb2222-0000-0000-0000-000000000002";

    fn realistic(i: usize) -> Value {
        json!({
            "sessionId": format!("local_{i:08}-1111-2222-3333-444455556666"),
            "cliSessionId": format!("{i:08}-aaaa-bbbb-cccc-ddddeeeeffff"),
            "cwd": "C:\\Users\\test\\proj", "originCwd": "C:\\Users\\test\\proj",
            "title": "Bir sohbet", "createdAt": 1762700000000i64, "lastActivityAt": 1762700500000i64,
            "completedTurns": 4, "model": "claude-opus-4-6", "isArchived": false, "isStarred": false,
            "bridgeSessionIds": [], "permissionMode": "default", "rewindEdges": [],
            "spawnedFrom": null, "someFutureKey": {"x": 1}
        })
    }

    fn write_all(root: &Path, files: &[Value]) -> PathBuf {
        let dir = root.join("claude-code-sessions").join(ACC).join(ORG);
        fs::create_dir_all(&dir).unwrap();
        for (i, v) in files.iter().enumerate() {
            fs::write(dir.join(format!("local_{i}.json")), v.to_string()).unwrap();
        }
        dir
    }

    #[test]
    fn passes_on_realistic_and_fixture_shaped_pointers() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_all(t.path(), &(0..6).map(realistic).collect::<Vec<_>>());
        assert_eq!(check(t.path()), None);
        // a couple of individually broken files are tolerated
        fs::write(dir.join("local_broken.json"), "{ not json").unwrap();
        fs::write(dir.join("local_array.json"), "[]").unwrap();
        assert_eq!(check(t.path()), None);

        // freshly opened chats have no title yet: 5 newest without one must not trip
        let t = tempfile::tempdir().unwrap();
        let untitled: Vec<Value> = (0..5)
            .map(|i| {
                let mut v = realistic(i);
                v.as_object_mut().unwrap().remove("title");
                v
            })
            .collect();
        write_all(t.path(), &untitled);
        assert_eq!(check(t.path()), None);

        // the minimal shape sync_engine's own tests use (non-uuid org folder too)
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("claude-code-sessions").join(ACC).join("org-aaaa");
        fs::create_dir_all(&dir).unwrap();
        for sid in ["picked", "grouped", "child", "other"] {
            let v = json!({"sessionId": format!("local_{sid}"), "cliSessionId": format!("cli-{sid}"), "title": sid, "createdAt": 1, "lastActivityAt": 2});
            fs::write(dir.join(format!("local_{sid}.json")), v.to_string()).unwrap();
        }
        let cfg = json!({"preferences": {"epitaxyPrefs": {"dframe-group-scopes": {format!("{ACC}/org-aaaa"): {
            "groups": [{"id": "cg-1", "name": "Bölüm"}], "assignments": {"code:local_grouped": "cg-1"}, "order": {}
        }}}}});
        fs::write(t.path().join("claude_desktop_config.json"), cfg.to_string()).unwrap();
        assert_eq!(check(t.path()), None);
    }

    #[test]
    fn trips_on_changed_format() {
        // cliSessionId renamed in every file
        let t = tempfile::tempdir().unwrap();
        let renamed: Vec<Value> = (0..4)
            .map(|i| {
                let mut v = realistic(i);
                let id = v.as_object_mut().unwrap().remove("cliSessionId").unwrap();
                v["cliSession"] = id;
                v
            })
            .collect();
        write_all(t.path(), &renamed);
        assert!(check(t.path()).unwrap().contains("cliSessionId"));

        // most files broken
        let t = tempfile::tempdir().unwrap();
        let dir = write_all(t.path(), &[realistic(0)]);
        for i in 0..4 {
            fs::write(dir.join(format!("local_x{i}.json")), "[]").unwrap();
        }
        assert!(check(t.path()).is_some());

        // pointers moved one level up (no org folder)
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("claude-code-sessions").join(ACC);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("local_a.json"), realistic(0).to_string()).unwrap();
        assert!(check(t.path()).is_some());

        // config top level / sidebar scopes changed
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("claude_desktop_config.json"), "[]").unwrap();
        assert!(check(t.path()).is_some());
        let cfg = json!({"preferences": {"epitaxyPrefs": {"dframe-group-scopes": [{"groups": []}]}}});
        fs::write(t.path().join("claude_desktop_config.json"), cfg.to_string()).unwrap();
        assert!(check(t.path()).is_some());
    }

    #[test]
    fn small_samples_never_trip() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_all(t.path(), &[]);
        fs::write(dir.join("local_a.json"), "[]").unwrap();
        fs::write(dir.join("local_b.json"), "{}").unwrap();
        assert_eq!(check(t.path()), None);
    }
}
