//! Token spending per project, chat and model, read from Claude Code transcripts
//! (`~/.claude/projects/<project>/<cliSessionId>.jsonl`, plus anything nested under
//! `<cliSessionId>/`, such as subagent transcripts). Transcripts are only read.
//!
//! Transcripts are append-only, so each file is parsed once and later only from the byte where
//! the last read stopped. Usage is summed per file into 10-minute slots (with the 48-bit hashes
//! of the message ids, for the cross-file dedupe) and kept in `<studio_dir>/spending-cache.json`,
//! so a restart does not rescan. One scan runs at a time; changed files are read in parallel.

use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager, State};

const DAY_MS: i64 = 86_400_000;
/// Cache granularity. The window start is rounded down to a slot (at most 10 extra minutes).
const SLOT_MS: i64 = 600_000;
const MAX_FILE_BYTES: u64 = 300 * 1024 * 1024;
/// Largest window; records of files untouched for longer are pruned.
const MAX_DAYS: u32 = 90;
const TOP: usize = 15;
const UNTITLED: &str = "Adsız sohbet";
const UNTITLED_EN: &str = "Untitled chat";
const CACHE_FILE: &str = "spending-cache.json";
const CACHE_VERSION: u32 = 1;
const WARMUP_DELAY: Duration = Duration::from_secs(60);
const WARMUP_DAYS: u32 = 30;
const ID_MASK: u64 = (1 << 48) - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSums {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub total: u64,
    /// input + output + cache write: what the turns processed, without the conversation re-read
    /// from cache every turn. Rankings and shares use this.
    pub work: u64,
}

impl TokenSums {
    fn add(&mut self, o: &TokenSums) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.total += o.total;
        self.work += o.work;
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
    pub work: u64,
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

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: usize,
}

/// Usage of one file in one 10-minute slot, for one (model, cwd).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Slot {
    /// Timestamp / SLOT_MS.
    s: i64,
    m: String,
    c: String,
    /// Input, output, cache write, cache read.
    t: [u64; 4],
    /// Messages, with or without an id.
    n: u64,
    /// Newest timestamp (ms).
    l: i64,
    /// 48-bit hashes of the message ids.
    #[serde(with = "ids")]
    i: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct FileRec {
    mtime: i64,
    size: u64,
    /// Bytes parsed so far (whole lines only).
    off: u64,
    /// Hash of the first and the last (up to) 64 bytes before `off`; a different value means the
    /// file was replaced.
    tail: u64,
    /// cwd of the last usage line before `off`: later lines without one inherit it.
    cwd: String,
    slots: Vec<Slot>,
}

#[derive(Default)]
struct Store {
    files: HashMap<PathBuf, FileRec>,
    loaded: bool,
}

#[derive(Deserialize)]
struct Disk {
    v: u32,
    files: HashMap<PathBuf, FileRec>,
}

#[derive(Serialize)]
struct DiskRef<'a> {
    v: u32,
    files: HashMap<&'a str, &'a FileRec>,
}

/// Held for a whole scan + report, so only one scan runs at a time.
fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

/// Set once the page asked for a report: the warm-up then stops running at background priority.
static URGENT: AtomicBool = AtomicBool::new(false);

mod ids {
    use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine};

    pub fn serialize<S: serde::Serializer>(v: &[u64], s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes().into_iter().take(6)).collect();
        s.serialize_str(&B64.encode(bytes))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
        let raw = B64.decode(<String as serde::Deserialize>::deserialize(d)?).map_err(serde::de::Error::custom)?;
        if raw.len() % 6 != 0 {
            return Err(serde::de::Error::custom("bad id list"));
        }
        Ok(raw.chunks(6).map(|c| c.iter().rev().fold(0u64, |h, &b| h << 8 | b as u64)).collect())
    }
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn ms(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Deserialize)]
struct Line<'a> {
    #[serde(borrow)]
    cwd: Option<Cow<'a, str>>,
    #[serde(borrow, rename = "type")]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    timestamp: Option<Cow<'a, str>>,
    #[serde(borrow)]
    message: Option<Msg<'a>>,
}

#[derive(Deserialize)]
struct Msg<'a> {
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    model: Option<Cow<'a, str>>,
    usage: Option<Usage>,
}

/// `Value` per field so a null or odd count reads as 0, as before.
#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: Value,
    #[serde(default)]
    output_tokens: Value,
    #[serde(default)]
    cache_creation_input_tokens: Value,
    #[serde(default)]
    cache_read_input_tokens: Value,
}

/// One assistant usage line.
#[derive(Debug)]
struct Parsed<'a> {
    ts: i64,
    id: Option<u64>,
    model: Cow<'a, str>,
    t: [u64; 4],
}

fn entry<'a>(ts: &str, id: Option<&str>, model: Option<Cow<'a, str>>, t: [u64; 4]) -> Option<Parsed<'a>> {
    let ts = chrono::DateTime::parse_from_rfc3339(ts).ok()?.timestamp_millis();
    if t.iter().sum::<u64>() == 0 {
        return None;
    }
    let id = id.map(|i| fnv(i.as_bytes()) & ID_MASK);
    Some(Parsed { ts, id, model: model.unwrap_or(Cow::Borrowed("bilinmiyor")), t })
}

/// Usage of one transcript line; `cwd` follows the last line that names one.
fn parse_line<'a>(line: &'a [u8], cwd: &mut String) -> Option<Parsed<'a>> {
    static FINDER: OnceLock<memchr::memmem::Finder<'static>> = OnceLock::new();
    // Cheap pre-filter on raw bytes: most lines (user turns, tool results) carry no usage.
    FINDER.get_or_init(|| memchr::memmem::Finder::new(b"\"usage\"")).find(line)?;
    let Ok(l) = serde_json::from_slice::<Line>(line) else { return parse_value(line, cwd) };
    if let Some(c) = l.cwd {
        if *cwd != *c {
            *cwd = c.into_owned();
        }
    }
    if l.kind.as_deref() != Some("assistant") {
        return None;
    }
    let msg = l.message?;
    let u = msg.usage?;
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let t = [n(&u.input_tokens), n(&u.output_tokens), n(&u.cache_creation_input_tokens), n(&u.cache_read_input_tokens)];
    entry(l.timestamp.as_deref()?, msg.id.as_deref(), msg.model, t)
}

/// Slow path for lines the typed parse refuses (odd field types, invalid UTF-8, duplicate keys).
fn parse_value<'a>(line: &[u8], cwd: &mut String) -> Option<Parsed<'a>> {
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(line)).ok()?;
    if let Some(c) = v.get("cwd").and_then(Value::as_str) {
        if cwd != c {
            *cwd = c.to_string();
        }
    }
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let msg = v.get("message")?;
    let u = msg.get("usage")?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let t = [n("input_tokens"), n("output_tokens"), n("cache_creation_input_tokens"), n("cache_read_input_tokens")];
    let model = msg.get("model").and_then(Value::as_str).map(|m| Cow::Owned(m.to_string()));
    entry(v.get("timestamp")?.as_str()?, msg.get("id").and_then(Value::as_str), model, t)
}

fn add(rec: &mut FileRec, p: Parsed, seen: &mut HashSet<u64>) {
    // Claude Code writes one line per content block, all with the same id and usage.
    if p.id.is_some_and(|id| !seen.insert(id)) {
        return;
    }
    let s = p.ts.div_euclid(SLOT_MS);
    // Only the newest slots are checked; a repeated (slot, model, cwd) key is still summed correctly.
    let found = rec.slots.iter().rev().take(8).position(|x| x.s == s && x.m == p.model && x.c == rec.cwd);
    let slot = match found {
        Some(i) => {
            let len = rec.slots.len();
            &mut rec.slots[len - 1 - i]
        }
        None => {
            rec.slots.push(Slot { s, m: p.model.into_owned(), c: rec.cwd.clone(), ..Default::default() });
            rec.slots.last_mut().unwrap()
        }
    };
    for (a, b) in slot.t.iter_mut().zip(p.t) {
        *a += b;
    }
    slot.n += 1;
    slot.l = slot.l.max(p.ts);
    slot.i.extend(p.id);
}

fn tail_hash(f: &mut File, off: u64) -> std::io::Result<u64> {
    let k = off.min(64);
    let mut b = vec![0; 2 * k as usize];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut b[..k as usize])?;
    f.seek(SeekFrom::Start(off - k))?;
    f.read_exact(&mut b[k as usize..])?;
    Ok(fnv(&b))
}

/// Parses what was appended to `path` since `rec.off`, or the whole file when it shrank or was replaced.
fn update(path: &Path, rec: &mut FileRec, mtime: i64, size: u64) -> std::io::Result<()> {
    let mut f = File::open(path)?;
    if rec.off > size || (rec.off > 0 && tail_hash(&mut f, rec.off)? != rec.tail) {
        *rec = FileRec::default();
    }
    f.seek(SeekFrom::Start(rec.off))?;
    let mut seen: HashSet<u64> = rec.slots.iter().flat_map(|s| s.i.iter().copied()).collect();
    let mut r = BufReader::with_capacity(1 << 20, f);
    let (mut buf, mut off) = (Vec::new(), rec.off);
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        // An unterminated last line may still be being written: it waits for the next read
        // unless it is already complete JSON.
        if n == 0 || (buf.last() != Some(&b'\n') && serde_json::from_slice::<serde::de::IgnoredAny>(&buf).is_err()) {
            break;
        }
        off += n as u64;
        if let Some(p) = parse_line(&buf, &mut rec.cwd) {
            add(rec, p, &mut seen);
        }
    }
    rec.tail = tail_hash(r.get_mut(), off)?;
    (rec.off, rec.mtime, rec.size) = (off, mtime, size);
    Ok(())
}

struct FileInfo {
    path: PathBuf,
    /// A nested file (e.g. `<id>/subagents/agent-x.jsonl`) counts toward chat `<id>`.
    chat: String,
    mtime: i64,
    size: u64,
}

/// Every `.jsonl` under each project folder, recursively, in directory order.
fn walk(projects_dir: &Path) -> Vec<FileInfo> {
    let mut out = Vec::new();
    for proj in fs::read_dir(projects_dir).into_iter().flatten().flatten() {
        walk_dir(&proj.path(), None, &mut out);
    }
    out
}

fn walk_dir(dir: &Path, chat: Option<&str>, out: &mut Vec<FileInfo>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let (p, Ok(ft)) = (e.path(), e.file_type()) else { continue };
        let name = e.file_name();
        let Some(chat) = chat.or(name.to_str()) else { continue };
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            walk_dir(&p, Some(chat), out);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            // DirEntry metadata comes with the directory listing on Windows: no extra open.
            let meta = if ft.is_symlink() { fs::metadata(&p) } else { e.metadata() };
            let Some((mtime, size)) = meta.ok().and_then(|m| Some((ms(m.modified().ok()?), m.len()))) else { continue };
            out.push(FileInfo { chat: chat.trim_end_matches(".jsonl").to_string(), path: p, mtime, size });
        }
    }
}

struct Run<'a> {
    threads: usize,
    /// Warm-up: background CPU/IO priority until the page asks, and a pause while `op_lock` is busy.
    low: bool,
    pause: &'a (dyn Fn() + Sync),
    progress: &'a dyn Fn(usize, usize),
}

struct Scan {
    files: Vec<FileInfo>,
    failed: HashSet<PathBuf>,
    dirty: bool,
}

#[cfg(windows)]
fn set_background(on: bool) {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END};
    // SAFETY: plain Win32 call on the current thread's pseudo-handle.
    unsafe { SetThreadPriority(GetCurrentThread(), if on { THREAD_MODE_BACKGROUND_BEGIN } else { THREAD_MODE_BACKGROUND_END }) };
}

#[cfg(not(windows))]
fn set_background(_: bool) {}

fn threads(low: bool) -> usize {
    let n = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    if low { (n / 2).max(1) } else { n }
}

/// Prunes records of deleted or long-untouched files, then brings the records of the files changed
/// in the last `days` up to date.
fn refresh(store: &mut Store, projects_dir: &Path, days: u32, now: i64, max_bytes: u64, run: &Run) -> Scan {
    let files = walk(projects_dir);
    let keep_from = now - MAX_DAYS as i64 * DAY_MS;
    let live: HashSet<&Path> = files.iter().filter(|f| f.mtime >= keep_from).map(|f| f.path.as_path()).collect();
    let before = store.files.len();
    store.files.retain(|p, _| live.contains(p.as_path()));
    let mut dirty = store.files.len() != before;

    let start = now - days as i64 * DAY_MS;
    let changed: Vec<&FileInfo> = files
        .iter()
        .filter(|f| f.mtime >= start && f.size <= max_bytes)
        .filter(|f| store.files.get(&f.path).is_none_or(|r| (r.mtime, r.size) != (f.mtime, f.size)))
        .collect();
    let mut work: Vec<_> =
        changed.iter().map(|f| (f.path.clone(), store.files.remove(&f.path).unwrap_or_default(), f.mtime, f.size)).collect();
    // Smallest first, as workers pop from the end: the biggest files start first.
    work.sort_by_key(|w| w.3);
    let total = work.len();
    let mut failed = HashSet::new();
    if total > 0 {
        dirty = true;
        (run.progress)(0, total);
        let (queue, (tx, rx), low, pause) = (Mutex::new(work), mpsc::channel(), run.low, run.pause);
        std::thread::scope(|sc| {
            for _ in 0..run.threads.clamp(1, total) {
                let (tx, queue) = (tx.clone(), &queue);
                sc.spawn(move || {
                    let mut bg = false;
                    loop {
                        if low {
                            let want = !URGENT.load(Ordering::Relaxed);
                            if want != bg {
                                set_background(want);
                                bg = want;
                            }
                            if want {
                                pause();
                            }
                        }
                        let Some((path, mut rec, mtime, size)) = queue.lock().unwrap().pop() else { break };
                        let res = update(&path, &mut rec, mtime, size).map(|_| rec);
                        if tx.send((path, res)).is_err() {
                            break;
                        }
                    }
                    if bg {
                        set_background(false);
                    }
                });
            }
            drop(tx);
            let (mut done, mut shown) = (0, Instant::now());
            for (path, res) in rx {
                done += 1;
                match res {
                    Ok(rec) => {
                        store.files.insert(path, rec);
                    }
                    Err(_) => {
                        failed.insert(path);
                    }
                }
                if done == total || shown.elapsed() >= Duration::from_millis(100) {
                    (run.progress)(done, total);
                    shown = Instant::now();
                }
            }
        });
    }
    Scan { files, failed, dirty }
}

fn load(store: &mut Store, studio_dir: &Path) {
    if store.loaded {
        return;
    }
    store.loaded = true;
    // A missing, corrupt or old-format cache is simply rebuilt.
    store.files = fs::read(studio_dir.join(CACHE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice::<Disk>(&b).ok())
        .filter(|d| d.v == CACHE_VERSION)
        .map(|d| d.files)
        .unwrap_or_default();
}

fn save(store: &Store, studio_dir: &Path) -> Result<(), String> {
    let files = store.files.iter().filter_map(|(p, r)| Some((p.to_str()?, r))).collect();
    let bytes = serde_json::to_vec(&DiskRef { v: CACHE_VERSION, files }).map_err(|e| e.to_string())?;
    crate::dpapi::atomic_write_verified(&studio_dir.join(CACHE_FILE), &bytes).map(|_| ())
}

/// Token share of `keep` of the `of` messages in a slot (the rest were counted in an earlier file).
fn share(t: [u64; 4], keep: u64, of: u64) -> TokenSums {
    let f = |x: u64| (x as u128 * keep as u128 / of as u128) as u64;
    let (input, output, cache_write, cache_read) = (f(t[0]), f(t[1]), f(t[2]), f(t[3]));
    let work = input + output + cache_write;
    TokenSums { input, output, cache_write, cache_read, total: work + cache_read, work }
}

#[derive(Default)]
struct Acc<'a> {
    tokens: TokenSums,
    last: i64,
    chats: HashSet<&'a str>,
    cwd: &'a str,
}

/// `known`: cli session id -> (title, cwd) from the app's session list.
fn report(
    store: &Store,
    scan: &Scan,
    known: &HashMap<String, (String, Option<String>)>,
    days: u32,
    now: i64,
    max_bytes: u64,
) -> SpendingReport {
    let start = now - days as i64 * DAY_MS;
    let (first, last) = (start.div_euclid(SLOT_MS), now.div_euclid(SLOT_MS));
    let mut report = SpendingReport { days, ..Default::default() };
    let mut seen = HashSet::new();
    let (mut projects, mut chats, mut models) = (HashMap::<&str, Acc>::new(), HashMap::<&str, Acc>::new(), HashMap::<&str, (u64, u64)>::new());

    for f in scan.files.iter().filter(|f| f.mtime >= start) {
        if f.size > max_bytes || scan.failed.contains(&f.path) {
            report.skipped_files += 1;
            continue;
        }
        let Some(rec) = store.files.get(&f.path) else { continue };
        let known_cwd = known.get(&f.chat).and_then(|(_, c)| c.as_deref());
        for s in rec.slots.iter().filter(|s| (first..=last).contains(&s.s)) {
            // Cross-file dedupe (a resumed chat repeats earlier messages): the first file keeps them.
            // Duplicates in a slot that also has new messages are removed by their average size.
            let dup = s.i.iter().filter(|id| !seen.insert(**id)).count() as u64;
            let n = s.n - dup;
            if n == 0 {
                continue;
            }
            let t = share(s.t, n, s.n);
            let cwd = match known_cwd {
                Some(k) if s.c.is_empty() => k,
                _ => s.c.as_str(),
            };
            report.totals.tokens.add(&t);
            report.totals.messages += n;
            let m = models.entry(&s.m).or_default();
            (m.0, m.1) = (m.0 + t.total, m.1 + t.work);
            let p = projects.entry(cwd).or_default();
            p.tokens.add(&t);
            p.last = p.last.max(s.l);
            p.chats.insert(&f.chat);
            let c = chats.entry(&f.chat).or_default();
            c.tokens.add(&t);
            if s.l >= c.last {
                c.last = s.l;
                c.cwd = cwd;
            }
        }
    }

    report.totals.chats = chats.len() as u64;
    report.projects = projects
        .into_iter()
        .map(|(cwd, a)| ProjectSpend { cwd: cwd.to_string(), tokens: a.tokens, chats: a.chats.len() as u64, last_activity_at: a.last })
        .collect();
    report.projects.sort_by(|a, b| b.tokens.work.cmp(&a.tokens.work).then_with(|| a.cwd.cmp(&b.cwd)));
    report.projects.truncate(TOP);
    report.chats = chats
        .into_iter()
        .map(|(id, a)| {
            let (title, cwd) = match known.get(id) {
                Some((t, c)) => (t.clone(), c.clone().unwrap_or_else(|| a.cwd.to_string())),
                None => (crate::i18n::t(UNTITLED, UNTITLED_EN).to_string(), a.cwd.to_string()),
            };
            ChatSpend { cli_session_id: id.to_string(), title, cwd, tokens: a.tokens, last_activity_at: a.last }
        })
        .collect();
    report.chats.sort_by(|a, b| b.tokens.work.cmp(&a.tokens.work).then_with(|| a.cli_session_id.cmp(&b.cli_session_id)));
    report.chats.truncate(TOP);
    report.models = models.into_iter().map(|(m, (total, work))| ModelSpend { model: m.to_string(), total, work }).collect();
    report.models.sort_by(|a, b| b.work.cmp(&a.work).then_with(|| a.model.cmp(&b.model)));
    report
}

fn emit_progress(app: &AppHandle, done: usize, total: usize) {
    let _ = app.emit("spending-progress", Progress { done, total });
}

#[tauri::command(async)]
pub fn get_spending(days: u32, app: AppHandle, state: State<'_, AppState>) -> SpendingReport {
    let (projects, sessions) = crate::chat_text::listed_chats(&state);
    let known = sessions
        .into_iter()
        .filter_map(|s| Some((s.cli_session_id?, (s.title, s.cwd.or(s.origin_cwd)))))
        .collect();
    let days = days.clamp(1, MAX_DAYS);
    URGENT.store(true, Ordering::Relaxed);
    // A running warm-up holds the store: this waits for it and then only reads what is still missing.
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    load(&mut store, &state.studio_dir);
    let now = chrono::Utc::now().timestamp_millis();
    let progress = |d, t| emit_progress(&app, d, t);
    let scan = refresh(&mut store, &projects, days, now, MAX_FILE_BYTES, &Run { threads: threads(false), low: false, pause: &|| {}, progress: &progress });
    let r = report(&store, &scan, &known, days, now, MAX_FILE_BYTES);
    if scan.dirty {
        // Off the request path; the next scan waits for it on the store lock.
        let dir = state.studio_dir.clone();
        std::thread::spawn(move || {
            let _ = save(&self::store().lock().unwrap_or_else(|e| e.into_inner()), &dir);
        });
    }
    r
}

/// One low-priority scan a minute after start, so the page opens with the data already read.
/// It never takes `op_lock`; it only checks it with `try_lock` between files and waits while it is busy.
pub fn spawn_warmup(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(WARMUP_DELAY);
        let state = app.state::<AppState>();
        let pause = || {
            while matches!(state.op_lock.try_lock(), Err(TryLockError::WouldBlock)) {
                std::thread::sleep(Duration::from_millis(250));
            }
        };
        let progress = |d, t| emit_progress(&app, d, t);
        let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
        load(&mut store, &state.studio_dir);
        let now = chrono::Utc::now().timestamp_millis();
        let run = Run { threads: threads(!URGENT.load(Ordering::Relaxed)), low: true, pause: &pause, progress: &progress };
        if refresh(&mut store, &crate::chat_text::cli_projects_dir(), WARMUP_DAYS, now, MAX_FILE_BYTES, &run).dirty {
            let _ = save(&store, &state.studio_dir);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    /// Real "now": fixture files get the current mtime, which must fall inside the window.
    fn now() -> i64 {
        static NOW: OnceLock<i64> = OnceLock::new();
        *NOW.get_or_init(|| chrono::Utc::now().timestamp_millis())
    }

    fn iso(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms).unwrap().to_rfc3339()
    }

    fn asst_at(id: &str, ts: i64, cwd: &str, model: &str, inp: u64, out: u64, cw: u64, cr: u64) -> Value {
        json!({
            "type": "assistant",
            "timestamp": iso(ts),
            "cwd": cwd,
            "message": {"id": id, "model": model, "usage": {
                "input_tokens": inp, "output_tokens": out,
                "cache_creation_input_tokens": cw, "cache_read_input_tokens": cr
            }}
        })
    }

    fn asst(id: &str, ago_days: f64, cwd: &str, model: &str, inp: u64, out: u64, cw: u64, cr: u64) -> Value {
        asst_at(id, now() - (ago_days * DAY_MS as f64) as i64, cwd, model, inp, out, cw, cr)
    }

    fn text(lines: &[Value]) -> String {
        lines.iter().map(|l| l.to_string() + "\n").collect()
    }

    fn write(dir: &Path, rel: &str, lines: &[Value]) -> PathBuf {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text(lines)).unwrap();
        path
    }

    fn append(path: &Path, s: &str) {
        File::options().append(true).open(path).unwrap().write_all(s.as_bytes()).unwrap();
    }

    fn quiet(threads: usize) -> Run<'static> {
        Run { threads, low: false, pause: &|| {}, progress: &|_, _| {} }
    }

    fn scan_report(store: &mut Store, dir: &Path, known: &HashMap<String, (String, Option<String>)>, days: u32, max: u64, threads: usize) -> SpendingReport {
        let scan = refresh(store, dir, days, now(), max, &quiet(threads));
        report(store, &scan, known, days, now(), max)
    }

    fn run(dir: &Path, known: &HashMap<String, (String, Option<String>)>, days: u32, max: u64) -> SpendingReport {
        scan_report(&mut Store::default(), dir, known, days, max, 4)
    }

    fn total(store: &mut Store, dir: &Path) -> u64 {
        scan_report(store, dir, &HashMap::new(), 7, MAX_FILE_BYTES, 2).totals.tokens.total
    }

    /// The pre-cache algorithm: every line, exact dedupe by id in file order, exact window.
    fn exact_total(dir: &Path, days: u32) -> u64 {
        let start = now() - days as i64 * DAY_MS;
        let mut seen = HashSet::new();
        let mut sum = 0;
        for f in walk(dir) {
            let mut cwd = String::new();
            for line in fs::read(&f.path).unwrap().split_inclusive(|&b| b == b'\n') {
                if let Some(p) = parse_line(line, &mut cwd).filter(|p| p.ts >= start && p.ts <= now()) {
                    if p.id.is_none_or(|id| seen.insert(id)) {
                        sum += p.t.iter().sum::<u64>();
                    }
                }
            }
        }
        sum
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
        assert_eq!(r.totals.tokens, TokenSums { input: 516, output: 26, cache_write: 31, cache_read: 48, total: 621, work: 573 });
        assert_eq!((r.totals.messages, r.totals.chats, r.skipped_files), (5, 3, 0));

        assert_eq!(r.projects.iter().map(|p| (p.cwd.as_str(), p.tokens.work, p.tokens.total, p.chats)).collect::<Vec<_>>(), [("C:\\b", 500, 500, 1), ("C:\\a", 73, 121, 2)]);
        assert_eq!(r.projects[1].last_activity_at, now() - DAY_MS / 5);

        let chats: Vec<_> = r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.title.as_str(), c.tokens.work, c.tokens.total)).collect();
        assert_eq!(chats, [("c3", UNTITLED, 500, 500), ("c1", "Plan", 63, 104), ("c2", UNTITLED, 10, 17)]);
        assert_eq!(r.chats[0].cwd, "C:\\b");

        let models: Vec<_> = r.models.iter().map(|m| (m.model.as_str(), m.work, m.total)).collect();
        assert_eq!(models, [("sonnet", 503, 504), ("opus", 70, 110), ("haiku", 0, 7)]);

        // The 30-day window picks up the 10-day-old line too.
        assert_eq!(run(tmp.path(), &known, 30, MAX_FILE_BYTES).totals.tokens.total, 1621);
        assert_eq!(exact_total(tmp.path(), 7), 621);
    }

    #[test]
    fn rankings_use_work_tokens_not_cache_reads() {
        let tmp = tempfile::tempdir().unwrap();
        // A long chat: tiny turns re-reading a big conversation from cache.
        write(tmp.path(), "p1/long.jsonl", &[asst("l1", 1.0, "C:\\long", "opus", 1, 10, 0, 90_000), asst("l2", 1.0, "C:\\long", "opus", 1, 10, 0, 95_000)]);
        // A short chat that did real work.
        write(tmp.path(), "p2/busy.jsonl", &[asst("b1", 1.0, "C:\\busy", "sonnet", 200, 3_000, 5_000, 1_000)]);
        let r = run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES);
        assert_eq!((r.totals.tokens.work, r.totals.tokens.cache_read, r.totals.tokens.total), (8_222, 186_000, 194_222));
        assert_eq!(r.projects.iter().map(|p| p.cwd.as_str()).collect::<Vec<_>>(), ["C:\\busy", "C:\\long"]);
        assert_eq!(r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.tokens.work)).collect::<Vec<_>>(), [("busy", 8_200), ("long", 22)]);
        assert_eq!(r.models.iter().map(|m| (m.model.as_str(), m.work, m.total)).collect::<Vec<_>>(), [("sonnet", 8_200, 9_200), ("opus", 22, 185_022)]);
    }

    #[test]
    fn old_files_are_not_read_and_big_files_are_counted_as_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let old = write(tmp.path(), "p/old.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 9, 0, 0, 0)]);
        let ten_days_before_now = SystemTime::UNIX_EPOCH + Duration::from_millis((now() - 10 * DAY_MS) as u64);
        File::options().write(true).open(&old).unwrap().set_modified(ten_days_before_now).unwrap();
        let big = write(tmp.path(), "p/big.jsonl", &[asst("b", 1.0, "C:\\a", "opus", 7, 0, 0, 0)]);
        let mut store = Store::default();
        let r = scan_report(&mut store, tmp.path(), &HashMap::new(), 7, fs::metadata(&big).unwrap().len() - 1, 2);
        assert_eq!((r.totals.tokens.total, r.skipped_files), (0, 1));
        assert!(r.projects.is_empty() && r.chats.is_empty() && r.models.is_empty());
        assert!(store.files.is_empty(), "neither file was read");
    }

    #[test]
    fn appended_lines_are_read_incrementally_with_the_same_totals_as_a_full_read() {
        let tmp = tempfile::tempdir().unwrap();
        let first = asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0).to_string() + "\n";
        let path = write(tmp.path(), "p/c.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0)]);
        let mut store = Store::default();
        assert_eq!(total(&mut store, tmp.path()), 7);
        assert_eq!(store.files[&path].off, first.len() as u64);

        // Repeated id across the read boundary (second content block) plus new messages, one without cwd.
        let mut no_cwd = asst("c", 0.5, "", "sonnet", 100, 0, 0, 0);
        no_cwd.as_object_mut().unwrap().remove("cwd");
        append(&path, &text(&[asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0), asst("b", 1.0, "C:\\a", "opus", 10, 0, 0, 0), no_cwd]));
        let inc = scan_report(&mut store, tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES, 2);
        let full = run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES);
        assert_eq!(serde_json::to_value(&inc).unwrap(), serde_json::to_value(&full).unwrap());
        assert_eq!(inc.totals.tokens.total, 117);
        assert_eq!(inc.projects[0].cwd, "C:\\a", "the cwd carried over the read boundary");

        // Proof the old bytes were not read again: an in-place edit before the boundary goes unnoticed.
        let edited = fs::read_to_string(&path).unwrap().replacen("\"input_tokens\":3", "\"input_tokens\":9", 1);
        fs::write(&path, &edited).unwrap();
        append(&path, &text(&[asst("d", 0.1, "C:\\a", "opus", 1, 0, 0, 0)]));
        assert_eq!(total(&mut store, tmp.path()), 118);
        assert_eq!(run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES).totals.tokens.total, 124);
    }

    #[test]
    fn a_shrunk_or_replaced_file_is_read_again_from_the_start() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "p/c.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 4, 0, 0), asst("b", 1.0, "C:\\a", "opus", 10, 0, 0, 0)]);
        let mut store = Store::default();
        assert_eq!(total(&mut store, tmp.path()), 17);
        write(tmp.path(), "p/c.jsonl", &[asst("x", 1.0, "C:\\a", "opus", 5, 0, 0, 0)]);
        assert_eq!(total(&mut store, tmp.path()), 5);
        // Same length and the same last bytes before the boundary, other content, then grown.
        write(tmp.path(), "p/c.jsonl", &[asst("y", 1.0, "C:\\a", "opus", 6, 0, 0, 0), asst("z", 1.0, "C:\\a", "opus", 1, 0, 0, 0)]);
        assert_eq!(total(&mut store, tmp.path()), 7);
        assert_eq!(store.files[&path].off, fs::metadata(&path).unwrap().len());
    }

    #[test]
    fn an_unfinished_last_line_waits_until_it_is_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write(tmp.path(), "p/c.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 0, 0, 0)]);
        let line = asst("b", 1.0, "C:\\a", "opus", 50, 0, 0, 0).to_string();
        let (head, rest) = line.split_at(line.len() - 10);
        append(&path, head);
        let mut store = Store::default();
        assert_eq!(total(&mut store, tmp.path()), 3);
        append(&path, rest);
        // Complete but still without a newline: counted, as a full read would.
        assert_eq!(total(&mut store, tmp.path()), 53);
        append(&path, &("\n".to_string() + &text(&[asst("c", 1.0, "C:\\a", "opus", 100, 0, 0, 0)])));
        assert_eq!(total(&mut store, tmp.path()), 153);
    }

    #[test]
    fn the_cache_round_trips_and_a_corrupt_or_old_one_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, studio) = (tmp.path().join("projects"), tmp.path().join("studio"));
        write(&dir, "p/c1.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 3, 4, 5, 6), asst("b", 2.0, "C:\\b", "sonnet", 1, 0, 0, 0)]);
        let gone = write(&dir, "p/c2.jsonl", &[asst("c", 1.0, "C:\\a", "opus", 9, 0, 0, 0)]);
        let mut store = Store::default();
        let first = scan_report(&mut store, &dir, &HashMap::new(), 7, MAX_FILE_BYTES, 2);
        save(&store, &studio).unwrap();

        let mut again = Store::default();
        load(&mut again, &studio);
        assert_eq!(again.files, store.files);
        let scan = refresh(&mut again, &dir, 7, now(), MAX_FILE_BYTES, &quiet(2));
        assert!(!scan.dirty, "nothing was read after the restart");
        let second = report(&again, &scan, &HashMap::new(), 7, now(), MAX_FILE_BYTES);
        assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&second).unwrap());

        // A deleted transcript's record is pruned.
        fs::remove_file(&gone).unwrap();
        assert!(refresh(&mut again, &dir, 7, now(), MAX_FILE_BYTES, &quiet(2)).dirty);
        assert!(!again.files.contains_key(&gone));

        for bad in ["{not json", r#"{"v":0,"files":{}}"#, r#"{"files":{}}"#] {
            fs::write(studio.join(CACHE_FILE), bad).unwrap();
            let mut fresh = Store::default();
            load(&mut fresh, &studio);
            assert!(fresh.files.is_empty());
            assert_eq!(total(&mut fresh, &dir), 19);
        }
    }

    #[test]
    fn parallel_and_serial_scans_agree() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..40 {
            let lines: Vec<_> = (0..30)
                .map(|j| asst(&format!("m{}", (i * 30 + j) % 700), (i + j) as f64 * 0.1, &format!("C:\\p{}", i % 5), ["opus", "sonnet"][j % 2], j as u64, i as u64, 7, 11))
                .collect();
            write(tmp.path(), &format!("p{}/c{i}.jsonl", i % 3), &lines);
        }
        let serial = scan_report(&mut Store::default(), tmp.path(), &HashMap::new(), 30, MAX_FILE_BYTES, 1);
        let parallel = scan_report(&mut Store::default(), tmp.path(), &HashMap::new(), 30, MAX_FILE_BYTES, 8);
        assert_eq!(serde_json::to_value(&serial).unwrap(), serde_json::to_value(&parallel).unwrap());
        assert_eq!(serial.totals.tokens.total, exact_total(tmp.path(), 30));
    }

    /// Where the 10-minute slots differ from an exact per-message count, measured.
    #[test]
    fn slot_cache_error_is_bounded_and_measured() {
        let slot0 = (now() - 2 * DAY_MS).div_euclid(SLOT_MS) * SLOT_MS;
        // A resumed chat repeats its history with the original ids and timestamps: whole slots are
        // duplicates, so the dedupe stays exact.
        let tmp = tempfile::tempdir().unwrap();
        let history: Vec<_> = (0..20).map(|i| asst_at(&format!("h{i}"), slot0 + i * 60_000, "C:\\a", "opus", 10 * i as u64, 5, 100, 1000)).collect();
        write(tmp.path(), "p/a.jsonl", &history);
        let mut resumed = history.clone();
        resumed.extend((0..5).map(|i| asst_at(&format!("n{i}"), slot0 + 3 * SLOT_MS + i * 60_000, "C:\\a", "opus", 7, 7, 7, 7)));
        write(tmp.path(), "p/b.jsonl", &resumed);
        assert_eq!(run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES).totals.tokens.total, exact_total(tmp.path(), 7));

        // Worst case: new messages share a slot with repeated ones of a very different size; the
        // repeated ones are then removed by the slot's average.
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/a.jsonl", &[asst_at("x", slot0 + 60_000, "C:\\a", "opus", 0, 0, 0, 1000)]);
        write(tmp.path(), "p/b.jsonl", &[asst_at("x", slot0 + 60_000, "C:\\a", "opus", 0, 0, 0, 1000), asst_at("y", slot0 + 120_000, "C:\\a", "opus", 0, 0, 0, 10)]);
        let (got, exact) = (run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES).totals.tokens.total, exact_total(tmp.path(), 7));
        println!("mixed slot: cached {got}, exact {exact}, error {:+.1}%", (got as f64 / exact as f64 - 1.0) * 100.0);
        assert_eq!((got, exact), (1505, 1010));

        // Window start: the slot it falls in counts whole (here a message 5 minutes before it).
        let tmp = tempfile::tempdir().unwrap();
        let start = now() - 7 * DAY_MS;
        let before = start.div_euclid(SLOT_MS) * SLOT_MS + (start.rem_euclid(SLOT_MS) / 2);
        write(tmp.path(), "p/a.jsonl", &[asst_at("early", before, "C:\\a", "opus", 4, 0, 0, 0), asst("in", 1.0, "C:\\a", "opus", 1, 0, 0, 0)]);
        let got = run(tmp.path(), &HashMap::new(), 7, MAX_FILE_BYTES).totals.tokens.total;
        let exact = exact_total(tmp.path(), 7);
        assert!(exact == 1 && (got == 1 || got == 5), "at most one slot (10 min) extra: cached {got}, exact {exact}");
    }

    #[test]
    fn lines_without_usage_or_cwd_are_handled() {
        let mut cwd = String::new();
        assert!(parse_line(br#"{"type":"user","cwd":"C:\\y","message":{"content":"a"}}"#, &mut cwd).is_none());
        // Zero usage gives no entry but still sets the cwd.
        assert!(parse_line(asst("z", 0.0, "C:\\x", "opus", 0, 0, 0, 0).to_string().as_bytes(), &mut cwd).is_none());
        assert_eq!(cwd, "C:\\x");
        let mut line = asst("z", 0.0, "C:\\x", "opus", 1, 0, 0, 0);
        line.as_object_mut().unwrap().remove("cwd");
        // A line with no cwd inherits the previous line's.
        assert!(parse_line(line.to_string().as_bytes(), &mut cwd).is_some());
        assert_eq!(cwd, "C:\\x");
        assert!(parse_line(b"not json \"usage\"", &mut cwd).is_none());
        // Odd types take the slow path with the old rules: a numeric id counts as no id, a null count as 0.
        let odd = br#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"id":5,"model":"m","usage":{"input_tokens":null,"output_tokens":3}}}"#;
        let p = parse_line(odd, &mut cwd).unwrap();
        assert_eq!((p.id, p.t, p.model.as_ref()), (None, [0, 3, 0, 0], "m"));
        // Invalid UTF-8 is read lossily, as before.
        let mut bad = br#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","cwd":"C:\\"#.to_vec();
        bad.extend([0xff]);
        bad.extend(br#"","message":{"usage":{"output_tokens":2}}}"#);
        assert_eq!(parse_line(&bad, &mut cwd).unwrap().t, [0, 2, 0, 0]);
        assert_eq!(cwd, "C:\\\u{fffd}");
    }

    #[test]
    fn id_lists_round_trip_through_base64() {
        let s = Slot { i: vec![0, 1, ID_MASK, 0x1234_5678_9abc], ..Default::default() };
        let back: Slot = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    /// `cargo test --release spending_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn spending_benchmark() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, studio) = (tmp.path().join("projects"), tmp.path().join("studio"));
        let filler = "Lorem ipsum dolor sit amet, \\\"quoted\\\" text with a path C:\\\\x\\\\y.rs and a line\\nbreak. ".repeat(40);
        let (mut bytes, mut seed) = (0u64, 7u64);
        let mut rnd = |m: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        let mut paths = Vec::new();
        for f in 0..360 {
            let mut s = String::new();
            let lines = 150 + rnd(600);
            // A chat is chronological: one message every 5-35 s from a random start in the last 6 days.
            let mut at = now() - DAY_MS / 2 - rnd(5 * DAY_MS as u64) as i64;
            for j in 0..lines {
                at += 5_000 + rnd(30_000) as i64;
                let ts = iso(at);
                s += &format!(r#"{{"type":"user","timestamp":"{ts}","cwd":"C:\\p{}","message":{{"role":"user","content":[{{"type":"tool_result","content":"{filler}{filler}"}}]}}}}"#, f % 20);
                s += "\n";
                for block in 0..2 {
                    s += &format!(
                        r#"{{"parentUuid":"x","cwd":"C:\\p{}","type":"assistant","timestamp":"{ts}","message":{{"id":"msg_{f}_{j}","type":"message","role":"assistant","model":"claude-opus-4","content":[{{"type":"text","text":"{filler} {block}"}}],"stop_reason":null,"usage":{{"input_tokens":{},"output_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{},"service_tier":"standard"}}}},"requestId":"req_{f}_{j}"}}"#,
                        f % 20, rnd(50), rnd(2000), rnd(5000), rnd(200_000)
                    );
                    s += "\n";
                }
            }
            let path = dir.join(format!("p{}", f % 20)).join(format!("chat-{f}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &s).unwrap();
            bytes += s.len() as u64;
            paths.push(path);
        }
        let mb = bytes as f64 / 1e6;
        let time = |label: &str, store: &mut Store, threads: usize| {
            let t = Instant::now();
            let scan = refresh(store, &dir, 7, now(), MAX_FILE_BYTES, &quiet(threads));
            let r = report(store, &scan, &HashMap::new(), 7, now(), MAX_FILE_BYTES);
            let el = t.elapsed();
            println!("{label:<28} {:>8.0} ms  ({} messages, {} tokens)", el.as_secs_f64() * 1000.0, r.totals.messages, r.totals.tokens.total);
            (r, scan.dirty)
        };
        println!("fabricated: {} files, {mb:.0} MB", paths.len());
        let (serial, _) = time("cold, 1 thread", &mut Store::default(), 1);
        let mut store = Store::default();
        let (cold, _) = time(&format!("cold, {} threads", threads(false)), &mut store, threads(false));
        assert_eq!(serde_json::to_value(&serial).unwrap(), serde_json::to_value(&cold).unwrap());
        let (_, dirty) = time("warm, nothing changed", &mut store, threads(false));
        assert!(!dirty);
        for p in paths.iter().take(5) {
            append(p, &text(&[asst(&format!("new-{}", p.display()), 0.0, "C:\\p0", "claude-opus-4", 1, 2, 3, 4)]));
        }
        let (warm, _) = time("warm, 5 files appended", &mut store, threads(false));
        assert_eq!(warm.totals.tokens.total, cold.totals.tokens.total + 50);

        let t = Instant::now();
        save(&store, &studio).unwrap();
        let size = fs::metadata(studio.join(CACHE_FILE)).unwrap().len();
        println!("save cache                   {:>8.0} ms  ({:.1} MB, {:.1} bytes per message)", t.elapsed().as_secs_f64() * 1000.0, size as f64 / 1e6, size as f64 / warm.totals.messages as f64);
        let t = Instant::now();
        let mut restarted = Store::default();
        load(&mut restarted, &studio);
        println!("load cache (restart)         {:>8.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        let (after, dirty) = time("warm after restart", &mut restarted, threads(false));
        assert!(!dirty);
        assert_eq!(serde_json::to_value(&after).unwrap(), serde_json::to_value(&warm).unwrap());
        assert_eq!(warm.totals.tokens.total, exact_total(&dir, 7));
    }
}
