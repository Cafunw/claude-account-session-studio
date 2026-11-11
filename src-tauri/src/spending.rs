//! Token spending per project, chat and model, read from Claude Code transcripts
//! (`~/.claude/projects/<project>/<cliSessionId>.jsonl`, plus anything nested under
//! `<cliSessionId>/`, such as subagent transcripts). Read-only.

use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use tauri::State;

const DAY_MS: i64 = 86_400_000;
const MAX_FILE_BYTES: u64 = 300 * 1024 * 1024;
const TOP: usize = 15;
const UNTITLED: &str = "Adsız sohbet";
const UNTITLED_EN: &str = "Untitled chat";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSums {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub total: u64,
}

impl TokenSums {
    fn add(&mut self, o: &TokenSums) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.total += o.total;
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingTotals {
    pub tokens: TokenSums,
    pub messages: u64,
    pub chats: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpend {
    pub cwd: String,
    pub tokens: TokenSums,
    pub chats: u64,
    pub last_activity_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSpend {
    pub cli_session_id: String,
    pub title: String,
    pub cwd: String,
    pub tokens: TokenSums,
    pub last_activity_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    pub model: String,
    pub total: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingReport {
    pub days: u32,
    pub totals: SpendingTotals,
    pub projects: Vec<ProjectSpend>,
    pub chats: Vec<ChatSpend>,
    pub models: Vec<ModelSpend>,
    pub skipped_files: u64,
}

/// One assistant usage line.
#[derive(Debug)]
struct Entry {
    ts: i64,
    /// `message.id`: Claude Code writes one line per content block, all with the same id and usage.
    id: Option<Box<str>>,
    model: Arc<str>,
    cwd: Arc<str>,
    tokens: TokenSums,
}

type Cache = Mutex<HashMap<PathBuf, (SystemTime, u64, Arc<Vec<Entry>>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn parse_line(line: &str, last_cwd: &mut Arc<str>, last_model: &mut Arc<str>) -> Option<Entry> {
    // Cheap pre-filter: most lines (user turns, tool results) carry no usage.
    if !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if let Some(c) = v.get("cwd").and_then(Value::as_str) {
        if &**last_cwd != c {
            *last_cwd = c.into();
        }
    }
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let ts = chrono::DateTime::parse_from_rfc3339(v.get("timestamp")?.as_str()?).ok()?.timestamp_millis();
    let msg = v.get("message")?;
    let u = msg.get("usage")?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (input, output, cache_write, cache_read) =
        (n("input_tokens"), n("output_tokens"), n("cache_creation_input_tokens"), n("cache_read_input_tokens"));
    let total = input + output + cache_write + cache_read;
    if total == 0 {
        return None;
    }
    let model = msg.get("model").and_then(Value::as_str).unwrap_or("bilinmiyor");
    if &**last_model != model {
        *last_model = model.into();
    }
    Some(Entry {
        ts,
        id: msg.get("id").and_then(Value::as_str).map(Into::into),
        model: last_model.clone(),
        cwd: last_cwd.clone(),
        tokens: TokenSums { input, output, cache_write, cache_read, total },
    })
}

fn read_entries(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut reader = BufReader::new(File::open(path)?);
    let (mut buf, mut out) = (Vec::new(), Vec::new());
    let (mut cwd, mut model): (Arc<str>, Arc<str>) = ("".into(), "".into());
    while reader.read_until(b'\n', &mut buf)? > 0 {
        if let Some(e) = parse_line(&String::from_utf8_lossy(&buf), &mut cwd, &mut model) {
            out.push(e);
        }
        buf.clear();
    }
    Ok(out)
}

/// Parsed entries of `path`, reused while its (mtime, size) is unchanged.
fn load(cache: &Cache, path: &Path, mtime: SystemTime, size: u64) -> std::io::Result<Arc<Vec<Entry>>> {
    if let Some((m, s, e)) = cache.lock().unwrap().get(path) {
        if *m == mtime && *s == size {
            return Ok(e.clone());
        }
    }
    let entries = Arc::new(read_entries(path)?);
    cache.lock().unwrap().insert(path.to_path_buf(), (mtime, size, entries.clone()));
    Ok(entries)
}

/// Every `.jsonl` under `dir`, recursively.
fn jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            jsonl_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Acc {
    tokens: TokenSums,
    last: i64,
    chats: HashSet<String>,
    cwd: Arc<str>,
}

/// `known`: cli session id -> (title, cwd) from the app's session list.
fn build(
    projects_dir: &Path,
    known: &HashMap<String, (String, Option<String>)>,
    days: u32,
    now_ms: i64,
    max_bytes: u64,
    cache: &Cache,
) -> SpendingReport {
    let start = now_ms - days as i64 * DAY_MS;
    let mut report = SpendingReport { days, ..Default::default() };
    let mut seen_ids = HashSet::new();
    let (mut projects, mut chats, mut models) =
        (HashMap::<Arc<str>, Acc>::new(), HashMap::<String, Acc>::new(), HashMap::<Arc<str>, u64>::new());

    for proj in fs::read_dir(projects_dir).into_iter().flatten().flatten() {
        let proj_dir = proj.path();
        let mut files = Vec::new();
        jsonl_files(&proj_dir, &mut files);
        for path in files {
            let Ok(meta) = fs::metadata(&path) else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            let mtime_ms = mtime.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            if mtime_ms < start {
                continue;
            }
            if meta.len() > max_bytes {
                report.skipped_files += 1;
                continue;
            }
            let Ok(entries) = load(cache, &path, mtime, meta.len()) else {
                report.skipped_files += 1;
                continue;
            };
            // A nested file (e.g. `<id>/subagents/agent-x.jsonl`) counts toward chat `<id>`.
            let Some(chat_id) = path
                .strip_prefix(&proj_dir)
                .ok()
                .and_then(|r| r.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .map(|s| s.trim_end_matches(".jsonl").to_string())
            else {
                continue;
            };
            let known_cwd: Option<Arc<str>> = known.get(&chat_id).and_then(|(_, c)| c.as_deref()).map(Into::into);
            for e in entries.iter().filter(|e| e.ts >= start && e.ts <= now_ms) {
                if let Some(id) = &e.id {
                    if !seen_ids.insert(id.clone()) {
                        continue;
                    }
                }
                let cwd = if e.cwd.is_empty() { known_cwd.clone().unwrap_or_else(|| e.cwd.clone()) } else { e.cwd.clone() };
                report.totals.tokens.add(&e.tokens);
                report.totals.messages += 1;
                *models.entry(e.model.clone()).or_default() += e.tokens.total;
                let p = projects.entry(cwd.clone()).or_default();
                p.tokens.add(&e.tokens);
                p.last = p.last.max(e.ts);
                p.chats.insert(chat_id.clone());
                let c = chats.entry(chat_id.clone()).or_default();
                c.tokens.add(&e.tokens);
                if e.ts >= c.last {
                    c.last = e.ts;
                    c.cwd = cwd;
                }
            }
        }
    }

    report.totals.chats = chats.len() as u64;
    report.projects = projects
        .into_iter()
        .map(|(cwd, a)| ProjectSpend { cwd: cwd.to_string(), tokens: a.tokens, chats: a.chats.len() as u64, last_activity_at: a.last })
        .collect();
    report.projects.sort_by(|a, b| b.tokens.total.cmp(&a.tokens.total).then_with(|| a.cwd.cmp(&b.cwd)));
    report.projects.truncate(TOP);
    report.chats = chats
        .into_iter()
        .map(|(id, a)| {
            let (title, cwd) = match known.get(&id) {
                Some((t, c)) => (t.clone(), c.clone().unwrap_or_else(|| a.cwd.to_string())),
                None => (crate::i18n::t(UNTITLED, UNTITLED_EN).to_string(), a.cwd.to_string()),
            };
            ChatSpend { cli_session_id: id, title, cwd, tokens: a.tokens, last_activity_at: a.last }
        })
        .collect();
    report.chats.sort_by(|a, b| b.tokens.total.cmp(&a.tokens.total).then_with(|| a.cli_session_id.cmp(&b.cli_session_id)));
    report.chats.truncate(TOP);
    report.models = models.into_iter().map(|(m, total)| ModelSpend { model: m.to_string(), total }).collect();
    report.models.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.model.cmp(&b.model)));
    report
}

#[tauri::command(async)]
pub fn get_spending(days: u32, state: State<'_, AppState>) -> SpendingReport {
    let (projects, sessions) = crate::chat_text::listed_chats(&state);
    let known = sessions
        .into_iter()
        .filter_map(|s| Some((s.cli_session_id?, (s.title, s.cwd.or(s.origin_cwd)))))
        .collect();
    build(&projects, &known, days.clamp(1, 90), chrono::Utc::now().timestamp_millis(), MAX_FILE_BYTES, cache())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Real "now": fixture files get the current mtime, which must fall inside the window.
    fn now() -> i64 {
        static NOW: OnceLock<i64> = OnceLock::new();
        *NOW.get_or_init(|| chrono::Utc::now().timestamp_millis())
    }

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    fn asst(id: &str, ago_days: f64, cwd: &str, model: &str, inp: u64, out: u64, cw: u64, cr: u64) -> Value {
        json!({
            "type": "assistant",
            "timestamp": iso(now() - (ago_days * DAY_MS as f64) as i64),
            "cwd": cwd,
            "message": {"id": id, "model": model, "usage": {
                "input_tokens": inp, "output_tokens": out,
                "cache_creation_input_tokens": cw, "cache_read_input_tokens": cr
            }}
        })
    }

    fn write(dir: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        path
    }

    fn run(dir: &Path, known: &HashMap<String, (String, Option<String>)>, days: u32, max: u64) -> SpendingReport {
        build(dir, known, days, now(), max, &Cache::default())
    }

    #[test]
    fn sums_per_project_chat_and_model_inside_the_window_only() {
        let tmp = tempfile::tempdir().unwrap();
        let user = json!({"type": "user", "timestamp": iso(now()), "cwd": "C:\\a", "message": {"content": "selam"}});
        write(tmp.path(), "p1/c1.jsonl", &[
            user,
            asst("m1", 1.0, "C:\\a", "opus", 10, 20, 30, 40),
            // Same message id on a second content-block line: counted once.
            asst("m1", 1.0, "C:\\a", "opus", 10, 20, 30, 40),
            asst("m2", 2.0, "C:\\a", "sonnet", 1, 1, 1, 1),
            asst("old", 10.0, "C:\\a", "opus", 1000, 0, 0, 0),
        ]);
        write(tmp.path(), "p1/c2.jsonl", &[asst("m3", 0.5, "C:\\a", "opus", 5, 5, 0, 0)]);
        write(tmp.path(), "p2/c3.jsonl", &[asst("m4", 3.0, "C:\\b", "sonnet", 500, 0, 0, 0)]);
        // A subagent transcript nested under the chat folder counts toward that chat.
        write(tmp.path(), "p1/c2/subagents/agent-1.jsonl", &[asst("m5", 0.2, "C:\\a", "haiku", 0, 0, 0, 7)]);

        let known = HashMap::from([("c1".to_string(), ("Plan".to_string(), Some("C:\\a".to_string())))]);
        crate::i18n::set_test_lang(crate::i18n::Lang::Tr);
        let r = run(tmp.path(), &known, 7, MAX_FILE_BYTES);
        assert_eq!(r.totals.tokens, TokenSums { input: 516, output: 26, cache_write: 31, cache_read: 48, total: 621 });
        assert_eq!((r.totals.messages, r.totals.chats, r.skipped_files), (5, 3, 0));

        assert_eq!(r.projects.iter().map(|p| (p.cwd.as_str(), p.tokens.total, p.chats)).collect::<Vec<_>>(), [("C:\\b", 500, 1), ("C:\\a", 121, 2)]);
        assert_eq!(r.projects[1].last_activity_at, now() - DAY_MS / 5);

        let chats: Vec<_> = r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.title.as_str(), c.tokens.total)).collect();
        assert_eq!(chats, [("c3", UNTITLED, 500), ("c1", "Plan", 104), ("c2", UNTITLED, 17)]);
        assert_eq!(r.chats[0].cwd, "C:\\b");

        let models: Vec<_> = r.models.iter().map(|m| (m.model.as_str(), m.total)).collect();
        assert_eq!(models, [("sonnet", 504), ("opus", 110), ("haiku", 7)]);

        // The 30-day window picks up the 10-day-old line too.
        assert_eq!(run(tmp.path(), &known, 30, MAX_FILE_BYTES).totals.tokens.total, 1621);
    }

    #[test]
    fn old_files_are_not_read_and_big_files_are_counted_as_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let old = write(tmp.path(), "p/old.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 9, 0, 0, 0)]);
        let ten_days_before_now = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis((now() - 10 * DAY_MS) as u64);
        File::options().write(true).open(&old).unwrap().set_modified(ten_days_before_now).unwrap();
        let big = write(tmp.path(), "p/big.jsonl", &[asst("b", 1.0, "C:\\a", "opus", 7, 0, 0, 0)]);
        let r = run(tmp.path(), &HashMap::new(), 7, fs::metadata(&big).unwrap().len() - 1);
        assert_eq!((r.totals.tokens.total, r.skipped_files), (0, 1));
        assert!(r.projects.is_empty() && r.chats.is_empty() && r.models.is_empty());
    }

    #[test]
    fn unchanged_files_come_from_the_cache_with_the_same_totals() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "p/c.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0)]);
        let cache = Cache::default();
        let first = build(tmp.path(), &HashMap::new(), 7, now(), MAX_FILE_BYTES, &cache);
        let cached = cache.lock().unwrap().get(&path).unwrap().2.clone();
        let second = build(tmp.path(), &HashMap::new(), 7, now(), MAX_FILE_BYTES, &cache);
        assert!(Arc::ptr_eq(&cached, &cache.lock().unwrap().get(&path).unwrap().2), "second run re-read the file");
        assert_eq!(first.totals.tokens, second.totals.tokens);
        assert_eq!(second.totals.tokens.total, 7);

        // A changed file is read again.
        write(tmp.path(), "p/c.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0), asst("b", 1.0, "C:\\a", "opus", 10, 0, 0, 0)]);
        assert_eq!(build(tmp.path(), &HashMap::new(), 7, now(), MAX_FILE_BYTES, &cache).totals.tokens.total, 17);
    }

    #[test]
    fn lines_without_usage_or_cwd_are_handled() {
        let (mut cwd, mut model): (Arc<str>, Arc<str>) = ("".into(), "".into());
        assert!(parse_line(r#"{"type":"user","cwd":"C:\\y","message":{"content":"a"}}"#, &mut cwd, &mut model).is_none());
        // Zero usage gives no entry but still sets the cwd.
        assert!(parse_line(&asst("z", 0.0, "C:\\x", "opus", 0, 0, 0, 0).to_string(), &mut cwd, &mut model).is_none());
        assert_eq!(&*cwd, "C:\\x");
        let mut line = asst("z", 0.0, "C:\\x", "opus", 1, 0, 0, 0);
        line.as_object_mut().unwrap().remove("cwd");
        // A line with no cwd inherits the previous line's.
        assert_eq!(&*parse_line(&line.to_string(), &mut cwd, &mut model).unwrap().cwd, "C:\\x");
        assert!(parse_line("not json \"usage\"", &mut cwd, &mut model).is_none());
    }
}
