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
use chrono::{NaiveDate, TimeZone, Timelike};
use std::collections::{BTreeMap, HashMap, HashSet};
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
const UNTITLED: &str = "Adsız sohbet";
const UNTITLED_EN: &str = "Untitled chat";
const CACHE_FILE: &str = "spending-cache.json";
const CACHE_VERSION: u32 = 1;
const WARMUP_DELAY: Duration = Duration::from_secs(60);
const WARMUP_DAYS: i64 = 30;
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

/// API list prices in USD per million tokens: input, output, cache write, cache read.
/// Source: the claude-api skill's model table (cached 2026-11-03). Cache writes are the 5-minute
/// TTL rate (1.25x input; the 1-hour rate is 2x, which transcripts do not tell apart here); cache
/// reads are the per-model rate the skill lists, else 0.1x input. Haiku 5.5 uses its rate for prompts
/// up to 100K tokens. Models the skill gives no price for (4.5 and older, non-Claude) are unpriced.
pub const PRICES_AS_OF: &str = "2026-11-03";
const PRICES: &[(&str, [f64; 4])] = &[
    ("claude-fable-5-2", [8.0, 40.0, 10.0, 0.20]),
    ("claude-mythos-5-2", [8.0, 40.0, 10.0, 0.20]),
    ("claude-fable-5-1", [10.0, 50.0, 12.5, 0.25]),
    ("claude-mythos-5-1", [10.0, 50.0, 12.5, 0.25]),
    ("claude-fable-5", [10.0, 50.0, 12.5, 1.0]),
    ("claude-mythos-5", [10.0, 50.0, 12.5, 1.0]),
    ("claude-opus-5-5", [4.0, 20.0, 5.0, 0.20]),
    ("claude-opus-5", [5.0, 25.0, 6.25, 0.50]),
    ("claude-opus-4-8", [5.0, 25.0, 6.25, 0.50]),
    ("claude-opus-4-7", [5.0, 25.0, 6.25, 0.50]),
    ("claude-opus-4-6", [5.0, 25.0, 6.25, 0.50]),
    ("claude-sonnet-5-5", [2.0, 10.0, 2.5, 0.10]),
    ("claude-sonnet-5", [2.0, 10.0, 2.5, 0.20]),
    ("claude-sonnet-4-6", [3.0, 15.0, 3.75, 0.30]),
    ("claude-haiku-5-5", [0.10, 0.50, 0.125, 0.01]),
    ("claude-haiku-4-5", [1.0, 5.0, 1.25, 0.10]),
];

/// Price of a model id: the table id itself, or it followed by a date (`-20260101`), `@...` or `[1m]`.
fn price(model: &str) -> Option<[f64; 4]> {
    PRICES
        .iter()
        .find(|(id, _)| {
            model.strip_prefix(id).is_some_and(|rest| {
                rest.is_empty()
                    || rest.starts_with(['[', '@'])
                    || rest.strip_prefix('-').is_some_and(|d| d.len() >= 8 && d.bytes().take(8).all(|b| b.is_ascii_digit()))
            })
        })
        .map(|(_, p)| *p)
}

fn cost(t: &TokenSums, p: [f64; 4]) -> Cost {
    let f = |n: u64, per_m: f64| n as f64 * per_m / 1e6;
    let (input, output, cache_write, cache_read) = (f(t.input, p[0]), f(t.output, p[1]), f(t.cache_write, p[2]), f(t.cache_read, p[3]));
    Cost { input, output, cache_write, cache_read, total: input + output + cache_write + cache_read }
}

/// Model family for the chart: fable, mythos, opus, sonnet, haiku or other.
fn family(model: &str) -> &'static str {
    ["fable", "mythos", "opus", "sonnet", "haiku"].into_iter().find(|f| model.contains(f)).unwrap_or("other")
}

/// USD at API list prices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    pub total: f64,
}

impl Cost {
    fn add(&mut self, o: &Cost) {
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
    /// Of the priced models only.
    pub cost: Cost,
    pub messages: u64,
    pub chats: u64,
    /// Tokens of models with no list price, left out of `cost`.
    pub unpriced: TokenSums,
    /// The part that came from subagent transcripts.
    pub subagent_tokens: TokenSums,
    pub subagent_cost: f64,
}

/// The same span right before the window.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Previous {
    pub tokens: TokenSums,
    pub cost: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSpend {
    pub cwd: String,
    pub tokens: TokenSums,
    pub cost: f64,
    pub messages: u64,
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
    pub cost: f64,
    pub messages: u64,
    pub last_activity_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    pub model: String,
    pub family: &'static str,
    pub tokens: TokenSums,
    /// None: no list price for this model.
    pub cost: Option<f64>,
    pub messages: u64,
    pub last_activity_at: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Amount {
    pub work: u64,
    pub cost: f64,
}

impl Amount {
    fn add(&mut self, t: &TokenSums, usd: f64) {
        self.work += t.work;
        self.cost += usd;
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    /// Local midnight (ms).
    pub start: i64,
    pub families: BTreeMap<&'static str, Amount>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceRow {
    pub model: &'static str,
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingReport {
    /// "today" | "7" | "30" | "90" | "all"
    pub range: String,
    /// Local midnight of the window's first day (ms).
    pub start: i64,
    pub totals: SpendingTotals,
    /// None for all time.
    pub previous: Option<Previous>,
    /// All rows, most work first; the page shows the top ones.
    pub projects: Vec<ProjectSpend>,
    pub chats: Vec<ChatSpend>,
    pub models: Vec<ModelSpend>,
    /// One per local day of the window, oldest first.
    pub days: Vec<Day>,
    /// By local hour of day, 0-23.
    pub hours: Vec<Amount>,
    pub prices: Vec<PriceRow>,
    pub prices_as_of: &'static str,
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
    /// Subagent/workflow transcript: the file sits under a `subagents` folder. Its usage is still
    /// attributed to the parent chat, the session folder that holds that `subagents` folder.
    subagent: bool,
    mtime: i64,
    size: u64,
}

/// Every `.jsonl` under each project folder, recursively, in directory order.
fn walk(projects_dir: &Path) -> Vec<FileInfo> {
    let mut out = Vec::new();
    for proj in fs::read_dir(projects_dir).into_iter().flatten().flatten() {
        walk_dir(&proj.path(), None, false, &mut out);
    }
    out
}

fn walk_dir(dir: &Path, chat: Option<&str>, subagent: bool, out: &mut Vec<FileInfo>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let (p, Ok(ft)) = (e.path(), e.file_type()) else { continue };
        let name = e.file_name();
        let Some(chat) = chat.or(name.to_str()) else { continue };
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            walk_dir(&p, Some(chat), subagent || name == "subagents", out);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            // DirEntry metadata comes with the directory listing on Windows: no extra open.
            let meta = if ft.is_symlink() { fs::metadata(&p) } else { e.metadata() };
            let Some((mtime, size)) = meta.ok().and_then(|m| Some((ms(m.modified().ok()?), m.len()))) else { continue };
            out.push(FileInfo { chat: chat.trim_end_matches(".jsonl").to_string(), subagent, path: p, mtime, size });
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

/// Prunes records of deleted files, then brings the records of the files changed since `start`
/// (ms; `i64::MIN` for all) up to date.
fn refresh(store: &mut Store, projects_dir: &Path, start: i64, max_bytes: u64, run: &Run) -> Scan {
    let files = walk(projects_dir);
    let live: HashSet<&Path> = files.iter().map(|f| f.path.as_path()).collect();
    let before = store.files.len();
    store.files.retain(|p, _| live.contains(p.as_path()));
    let mut dirty = store.files.len() != before;

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
    cost: f64,
    messages: u64,
    last: i64,
    chats: HashSet<&'a str>,
    cwd: &'a str,
}

impl Acc<'_> {
    fn add(&mut self, t: &TokenSums, usd: f64, n: u64, last: i64) {
        self.tokens.add(t);
        self.cost += usd;
        self.messages += n;
        self.last = self.last.max(last);
    }
}

fn local<Tz: TimeZone>(tz: &Tz, ms: i64) -> chrono::DateTime<Tz> {
    tz.timestamp_millis_opt(ms).unwrap()
}

fn midnight<Tz: TimeZone>(tz: &Tz, d: NaiveDate) -> i64 {
    let t = d.and_time(chrono::NaiveTime::MIN);
    tz.from_local_datetime(&t).earliest().map_or_else(|| t.and_utc().timestamp_millis(), |x| x.timestamp_millis())
}

/// Window start (local midnight of its first day; `i64::MIN` for all time) and the shift back to the
/// previous period of the same length (the same hours of the earlier days, so a partial today
/// compares with the same part of yesterday).
fn window<Tz: TimeZone>(range: &str, now: i64, tz: &Tz) -> (i64, Option<i64>) {
    let days = match range {
        "today" => 1,
        "30" => 30,
        "90" => 90,
        "all" => return (i64::MIN, None),
        _ => 7,
    };
    let today = local(tz, now).date_naive();
    (midnight(tz, today - chrono::Days::new(days - 1)), Some(days as i64 * DAY_MS))
}

/// First byte time the scan must cover: the window and the previous period.
fn scan_from<Tz: TimeZone>(range: &str, now: i64, tz: &Tz) -> i64 {
    let (start, shift) = window(range, now, tz);
    shift.map_or(i64::MIN, |s| start - s)
}

/// Calls `f` for every slot in [start, end] (ms) with its tokens after the cross-file dedupe, and
/// returns how many files were skipped. A resumed chat repeats earlier messages: the first file in
/// walk order keeps them; duplicates in a slot that also has new messages are removed by their
/// average size.
fn each_slot<'a>(store: &'a Store, scan: &'a Scan, max_bytes: u64, start: i64, end: i64, mut f: impl FnMut(&'a FileInfo, &'a Slot, TokenSums, u64)) -> u64 {
    let (first, last) = (start.div_euclid(SLOT_MS), end.div_euclid(SLOT_MS));
    let (mut seen, mut skipped) = (HashSet::new(), 0);
    for fi in scan.files.iter().filter(|f| f.mtime >= start) {
        if fi.size > max_bytes || scan.failed.contains(&fi.path) {
            skipped += 1;
            continue;
        }
        let Some(rec) = store.files.get(&fi.path) else { continue };
        for s in rec.slots.iter().filter(|s| (first..=last).contains(&s.s)) {
            let dup = s.i.iter().filter(|id| !seen.insert(**id)).count() as u64;
            let n = s.n - dup;
            if n > 0 {
                f(fi, s, share(s.t, n, s.n), n);
            }
        }
    }
    skipped
}

/// `known`: cli session id -> (title, cwd) from the app's session list; `tz` sets day and hour bounds.
fn report<Tz: TimeZone>(
    store: &Store,
    scan: &Scan,
    known: &HashMap<String, (String, Option<String>)>,
    range: &str,
    now: i64,
    tz: &Tz,
    max_bytes: u64,
) -> SpendingReport {
    let (mut start, shift) = window(range, now, tz);
    if shift.is_none() {
        let first = scan.files.iter().filter_map(|f| store.files.get(&f.path)).flat_map(|r| r.slots.iter().map(|s| s.s)).min();
        start = midnight(tz, local(tz, first.map_or(now, |s| (s * SLOT_MS).min(now))).date_naive());
    }
    let first_day = local(tz, start).date_naive();
    let n_days = (local(tz, now).date_naive() - first_day).num_days().max(0) as u64 + 1;
    let mut r = SpendingReport {
        range: match range {
            "today" | "30" | "90" | "all" => range,
            _ => "7",
        }
        .into(),
        start,
        days: (0..n_days).map(|i| Day { start: midnight(tz, first_day + chrono::Days::new(i)), ..Default::default() }).collect(),
        hours: vec![Amount::default(); 24],
        prices: PRICES.iter().map(|&(model, [input, output, cache_write, cache_read])| PriceRow { model, input, output, cache_write, cache_read }).collect(),
        prices_as_of: PRICES_AS_OF,
        ..Default::default()
    };
    let (mut projects, mut chats, mut models) = (HashMap::<&str, Acc>::new(), HashMap::<&str, Acc>::new(), HashMap::<&str, Acc>::new());
    let mut prices = HashMap::<&str, Option<[f64; 4]>>::new();
    let totals = &mut r.totals;
    let (days, hours) = (&mut r.days, &mut r.hours);
    r.skipped_files = each_slot(store, scan, max_bytes, start, now, |fi, s, t, n| {
        let c = prices.entry(&s.m).or_insert_with(|| price(&s.m)).map(|p| cost(&t, p));
        let usd = c.map_or(0.0, |c| c.total);
        totals.tokens.add(&t);
        totals.messages += n;
        match c {
            Some(c) => totals.cost.add(&c),
            None => totals.unpriced.add(&t),
        }
        if fi.subagent {
            totals.subagent_tokens.add(&t);
            totals.subagent_cost += usd;
        }
        let cwd = match known.get(&fi.chat).and_then(|(_, c)| c.as_deref()) {
            Some(k) if s.c.is_empty() => k,
            _ => s.c.as_str(),
        };
        models.entry(&s.m).or_default().add(&t, usd, n, s.l);
        let p = projects.entry(cwd).or_default();
        p.add(&t, usd, n, s.l);
        p.chats.insert(&fi.chat);
        let c = chats.entry(&fi.chat).or_default();
        let newer = s.l >= c.last;
        c.add(&t, usd, n, s.l);
        if newer {
            c.cwd = cwd;
        }
        let at = local(tz, s.s * SLOT_MS);
        if let Some(d) = days.get_mut((at.date_naive() - first_day).num_days() as usize) {
            d.families.entry(family(&s.m)).or_default().add(&t, usd);
        }
        hours[at.hour() as usize].add(&t, usd);
    });
    if let Some(shift) = shift {
        let mut prev = Previous::default();
        each_slot(store, scan, max_bytes, start - shift, now - shift, |_, s, t, _| {
            prev.tokens.add(&t);
            prev.cost += price(&s.m).map_or(0.0, |p| cost(&t, p).total);
        });
        r.previous = Some(prev);
    }

    r.totals.chats = chats.len() as u64;
    r.projects = projects
        .into_iter()
        .map(|(cwd, a)| ProjectSpend { cwd: cwd.to_string(), tokens: a.tokens, cost: a.cost, messages: a.messages, chats: a.chats.len() as u64, last_activity_at: a.last })
        .collect();
    r.projects.sort_by(|a, b| b.tokens.work.cmp(&a.tokens.work).then_with(|| a.cwd.cmp(&b.cwd)));
    r.chats = chats
        .into_iter()
        .map(|(id, a)| {
            let (title, cwd) = match known.get(id) {
                Some((t, c)) => (t.clone(), c.clone().unwrap_or_else(|| a.cwd.to_string())),
                None => (crate::i18n::t(UNTITLED, UNTITLED_EN).to_string(), a.cwd.to_string()),
            };
            ChatSpend { cli_session_id: id.to_string(), title, cwd, tokens: a.tokens, cost: a.cost, messages: a.messages, last_activity_at: a.last }
        })
        .collect();
    r.chats.sort_by(|a, b| b.tokens.work.cmp(&a.tokens.work).then_with(|| a.cli_session_id.cmp(&b.cli_session_id)));
    r.models = models
        .into_iter()
        .map(|(m, a)| ModelSpend {
            model: m.to_string(),
            family: family(m),
            tokens: a.tokens,
            cost: price(m).map(|_| a.cost),
            messages: a.messages,
            last_activity_at: a.last,
        })
        .collect();
    r.models.sort_by(|a, b| b.tokens.work.cmp(&a.tokens.work).then_with(|| a.model.cmp(&b.model)));
    r
}

fn range_label(range: &str) -> String {
    match range {
        "today" => "bugun".into(),
        "all" => "tum-zamanlar".into(),
        n => format!("{n}-gun"),
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Projects, chats and models of `r` as one CSV (UTF-8 with BOM so Excel reads Turkish letters).
fn csv(r: &SpendingReport) -> String {
    let t = crate::i18n::t;
    let head = [
        t("Bölüm", "Section"),
        t("Ad", "Name"),
        t("Klasör", "Folder"),
        t("İşlenen token", "Tokens processed"),
        t("Girdi", "Input"),
        t("Çıktı", "Output"),
        t("Önbelleğe yazma", "Cache write"),
        t("Önbellekten okuma", "Cache read"),
        t("API karşılığı (USD)", "API equivalent (USD)"),
        t("Mesaj", "Messages"),
        t("Son etkinlik", "Last activity"),
    ];
    let mut out = String::from("\u{feff}");
    out += &head.join(",");
    out.push('\n');
    let mut row = |kind: &str, name: &str, folder: &str, tk: &TokenSums, usd: Option<f64>, messages: u64, last: i64| {
        let when = chrono::DateTime::from_timestamp_millis(last).map(|d| d.to_rfc3339()).unwrap_or_default();
        let usd = usd.map(|c| format!("{c:.4}")).unwrap_or_default();
        let cells = [kind.to_string(), csv_field(name), csv_field(folder), tk.work.to_string(), tk.input.to_string(), tk.output.to_string(), tk.cache_write.to_string(), tk.cache_read.to_string(), usd, messages.to_string(), when];
        out += &cells.join(",");
        out.push('\n');
    };
    for p in &r.projects {
        row(t("Proje", "Project"), &p.cwd, &p.cwd, &p.tokens, Some(p.cost), p.messages, p.last_activity_at);
    }
    for c in &r.chats {
        row(t("Sohbet", "Chat"), &c.title, &c.cwd, &c.tokens, Some(c.cost), c.messages, c.last_activity_at);
    }
    for m in &r.models {
        row("Model", &m.model, "", &m.tokens, m.cost, m.messages, m.last_activity_at);
    }
    out
}

/// Writes `csv(r)` to `dir` as `claude-harcama-<range>-<date>.csv`, never over an existing file.
fn write_csv(r: &SpendingReport, dir: &Path, date: &str) -> Result<PathBuf, String> {
    let stem = format!("claude-harcama-{}-{date}", range_label(&r.range));
    let (mut file, path) = crate::chat_text::create_unique(dir, &stem, "csv")?;
    std::io::Write::write_all(&mut file, csv(r).as_bytes()).map_err(crate::chat_text::write_failed)?;
    Ok(path)
}

fn emit_progress(app: &AppHandle, done: usize, total: usize) {
    let _ = app.emit("spending-progress", Progress { done, total });
}

#[tauri::command(async)]
pub fn get_spending(range: String, app: AppHandle, state: State<'_, AppState>) -> SpendingReport {
    let (projects, sessions) = crate::chat_text::listed_chats(&state);
    let known = sessions
        .into_iter()
        .filter_map(|s| Some((s.cli_session_id?, (s.title, s.cwd.or(s.origin_cwd)))))
        .collect();
    URGENT.store(true, Ordering::Relaxed);
    // A running warm-up holds the store: this waits for it and then only reads what is still missing.
    let mut store = store().lock().unwrap_or_else(|e| e.into_inner());
    load(&mut store, &state.studio_dir);
    let now = chrono::Utc::now().timestamp_millis();
    let progress = |d, t| emit_progress(&app, d, t);
    let from = scan_from(&range, now, &chrono::Local);
    let scan = refresh(&mut store, &projects, from, MAX_FILE_BYTES, &Run { threads: threads(false), low: false, pause: &|| {}, progress: &progress });
    let r = report(&store, &scan, &known, &range, now, &chrono::Local, MAX_FILE_BYTES);
    if scan.dirty {
        // Off the request path; the next scan waits for it on the store lock.
        let dir = state.studio_dir.clone();
        std::thread::spawn(move || {
            let _ = save(&self::store().lock().unwrap_or_else(|e| e.into_inner()), &dir);
        });
    }
    r
}

/// Saves the report of `range` to the Downloads folder as CSV and returns the new file's path.
#[tauri::command(async)]
pub fn export_spending_csv(range: String, app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let r = get_spending(range, app, state);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    Ok(write_csv(&r, &crate::chat_text::downloads_dir(), &date)?.to_string_lossy().into_owned())
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
        if refresh(&mut store, &crate::chat_text::cli_projects_dir(), now - WARMUP_DAYS * DAY_MS, MAX_FILE_BYTES, &run).dirty {
            let _ = save(&store, &state.studio_dir);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};
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

    fn scan_report(store: &mut Store, dir: &Path, known: &HashMap<String, (String, Option<String>)>, range: &str, max: u64, threads: usize) -> SpendingReport {
        let scan = refresh(store, dir, scan_from(range, now(), &Utc), max, &quiet(threads));
        report(store, &scan, known, range, now(), &Utc, max)
    }

    fn run(dir: &Path, known: &HashMap<String, (String, Option<String>)>, range: &str, max: u64) -> SpendingReport {
        scan_report(&mut Store::default(), dir, known, range, max, 4)
    }

    fn total(store: &mut Store, dir: &Path) -> u64 {
        scan_report(store, dir, &HashMap::new(), "7", MAX_FILE_BYTES, 2).totals.tokens.total
    }

    /// The pre-cache algorithm: every line, exact dedupe by id in file order, exact window.
    fn exact_total(dir: &Path, range: &str) -> u64 {
        let start = window(range, now(), &Utc).0;
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
        let r = run(tmp.path(), &known, "7", MAX_FILE_BYTES);
        assert_eq!(r.totals.tokens, TokenSums { input: 516, output: 26, cache_write: 31, cache_read: 48, total: 621, work: 573 });
        assert_eq!((r.totals.messages, r.totals.chats, r.skipped_files), (5, 3, 0));

        assert_eq!(r.projects.iter().map(|p| (p.cwd.as_str(), p.tokens.work, p.tokens.total, p.chats)).collect::<Vec<_>>(), [("C:\\b", 500, 500, 1), ("C:\\a", 73, 121, 2)]);
        assert_eq!(r.projects[1].last_activity_at, now() - DAY_MS / 5);

        let chats: Vec<_> = r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.title.as_str(), c.tokens.work, c.tokens.total)).collect();
        assert_eq!(chats, [("c3", UNTITLED, 500, 500), ("c1", "Plan", 63, 104), ("c2", UNTITLED, 10, 17)]);
        assert_eq!(r.chats[0].cwd, "C:\\b");

        let models: Vec<_> = r.models.iter().map(|m| (m.model.as_str(), m.tokens.work, m.tokens.total)).collect();
        assert_eq!(models, [("sonnet", 503, 504), ("opus", 70, 110), ("haiku", 0, 7)]);

        // The 30-day window picks up the 10-day-old line too.
        assert_eq!(run(tmp.path(), &known, "30", MAX_FILE_BYTES).totals.tokens.total, 1621);
        assert_eq!(exact_total(tmp.path(), "7"), 621);
    }

    #[test]
    fn rankings_use_work_tokens_not_cache_reads() {
        let tmp = tempfile::tempdir().unwrap();
        // A long chat: tiny turns re-reading a big conversation from cache.
        write(tmp.path(), "p1/long.jsonl", &[asst("l1", 1.0, "C:\\long", "opus", 1, 10, 0, 90_000), asst("l2", 1.0, "C:\\long", "opus", 1, 10, 0, 95_000)]);
        // A short chat that did real work.
        write(tmp.path(), "p2/busy.jsonl", &[asst("b1", 1.0, "C:\\busy", "sonnet", 200, 3_000, 5_000, 1_000)]);
        let r = run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES);
        assert_eq!((r.totals.tokens.work, r.totals.tokens.cache_read, r.totals.tokens.total), (8_222, 186_000, 194_222));
        assert_eq!(r.projects.iter().map(|p| p.cwd.as_str()).collect::<Vec<_>>(), ["C:\\busy", "C:\\long"]);
        assert_eq!(r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.tokens.work)).collect::<Vec<_>>(), [("busy", 8_200), ("long", 22)]);
        assert_eq!(r.models.iter().map(|m| (m.model.as_str(), m.tokens.work, m.tokens.total)).collect::<Vec<_>>(), [("sonnet", 8_200, 9_200), ("opus", 22, 185_022)]);
    }

    #[test]
    fn old_files_are_not_read_and_big_files_are_counted_as_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let old = write(tmp.path(), "p/old.jsonl", &[asst("a", 1.0, "C:\\a", "opus", 9, 0, 0, 0)]);
        // Older than the 7-day window and the 7 days before it.
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_millis((now() - 20 * DAY_MS) as u64);
        File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        let big = write(tmp.path(), "p/big.jsonl", &[asst("b", 1.0, "C:\\a", "opus", 7, 0, 0, 0)]);
        let mut store = Store::default();
        let r = scan_report(&mut store, tmp.path(), &HashMap::new(), "7", fs::metadata(&big).unwrap().len() - 1, 2);
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
        let inc = scan_report(&mut store, tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES, 2);
        let full = run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES);
        assert_eq!(serde_json::to_value(&inc).unwrap(), serde_json::to_value(&full).unwrap());
        assert_eq!(inc.totals.tokens.total, 117);
        assert_eq!(inc.projects[0].cwd, "C:\\a", "the cwd carried over the read boundary");

        // Proof the old bytes were not read again: an in-place edit before the boundary goes unnoticed.
        let edited = fs::read_to_string(&path).unwrap().replacen("\"input_tokens\":3", "\"input_tokens\":9", 1);
        fs::write(&path, &edited).unwrap();
        append(&path, &text(&[asst("d", 0.1, "C:\\a", "opus", 1, 0, 0, 0)]));
        assert_eq!(total(&mut store, tmp.path()), 118);
        assert_eq!(run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES).totals.tokens.total, 124);
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
        let first = scan_report(&mut store, &dir, &HashMap::new(), "7", MAX_FILE_BYTES, 2);
        save(&store, &studio).unwrap();

        let mut again = Store::default();
        load(&mut again, &studio);
        assert_eq!(again.files, store.files);
        let scan = refresh(&mut again, &dir, scan_from("7", now(), &Utc), MAX_FILE_BYTES, &quiet(2));
        assert!(!scan.dirty, "nothing was read after the restart");
        let second = report(&again, &scan, &HashMap::new(), "7", now(), &Utc, MAX_FILE_BYTES);
        assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&second).unwrap());

        // A deleted transcript's record is pruned.
        fs::remove_file(&gone).unwrap();
        assert!(refresh(&mut again, &dir, scan_from("7", now(), &Utc), MAX_FILE_BYTES, &quiet(2)).dirty);
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
        let serial = scan_report(&mut Store::default(), tmp.path(), &HashMap::new(), "30", MAX_FILE_BYTES, 1);
        let parallel = scan_report(&mut Store::default(), tmp.path(), &HashMap::new(), "30", MAX_FILE_BYTES, 8);
        assert_eq!(serde_json::to_value(&serial).unwrap(), serde_json::to_value(&parallel).unwrap());
        assert_eq!(serial.totals.tokens.total, exact_total(tmp.path(), "30"));
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
        assert_eq!(run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES).totals.tokens.total, exact_total(tmp.path(), "7"));

        // Worst case: new messages share a slot with repeated ones of a very different size; the
        // repeated ones are then removed by the slot's average.
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/a.jsonl", &[asst_at("x", slot0 + 60_000, "C:\\a", "opus", 0, 0, 0, 1000)]);
        write(tmp.path(), "p/b.jsonl", &[asst_at("x", slot0 + 60_000, "C:\\a", "opus", 0, 0, 0, 1000), asst_at("y", slot0 + 120_000, "C:\\a", "opus", 0, 0, 0, 10)]);
        let (got, exact) = (run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES).totals.tokens.total, exact_total(tmp.path(), "7"));
        println!("mixed slot: cached {got}, exact {exact}, error {:+.1}%", (got as f64 / exact as f64 - 1.0) * 100.0);
        assert_eq!((got, exact), (1505, 1010));

        // Windows start at local midnight, a slot boundary, so the window edge is exact.
        let tmp = tempfile::tempdir().unwrap();
        let start = window("7", now(), &Utc).0;
        write(tmp.path(), "p/a.jsonl", &[asst_at("early", start - 60_000, "C:\\a", "opus", 4, 0, 0, 0), asst_at("first", start, "C:\\a", "opus", 1, 0, 0, 0)]);
        assert_eq!((run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES).totals.tokens.total, exact_total(tmp.path(), "7")), (1, 1));
    }

    #[test]
    fn prices_match_by_id_prefix_and_unknown_models_are_unpriced() {
        assert_eq!(price("claude-opus-4-8"), Some([5.0, 25.0, 6.25, 0.50]));
        assert_eq!(price("claude-opus-4-6-20260115"), price("claude-opus-4-6"));
        assert_eq!(price("claude-opus-5-5[1m]"), Some([4.0, 20.0, 5.0, 0.20]));
        // A longer id is never priced as its shorter prefix, nor the other way round.
        assert_eq!(price("claude-opus-5-5"), Some([4.0, 20.0, 5.0, 0.20]));
        assert_eq!(price("claude-opus-5"), Some([5.0, 25.0, 6.25, 0.50]));
        assert_eq!(price("claude-opus-5-9"), None);
        for unknown in ["claude-sonnet-4-5-20250929", "claude-opus-4-1-20250805", "gpt-4o", "<synthetic>", "bilinmiyor", ""] {
            assert_eq!(price(unknown), None, "{unknown}");
        }
        assert_eq!((family("claude-opus-4-8"), family("claude-haiku-5-5"), family("gpt-4o")), ("opus", "haiku", "other"));
    }

    #[test]
    fn cost_uses_the_four_prices_and_leaves_unpriced_tokens_out() {
        const M: u64 = 1_000_000;
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/c.jsonl", &[
            asst("a", 0.1, "C:\\a", "claude-opus-4-8", M, M, M, M),
            asst("b", 0.1, "C:\\a", "claude-sonnet-5-5-20261001", 2 * M, 0, 0, 10 * M),
            asst("c", 0.1, "C:\\a", "gpt-4o", 3 * M, 0, 0, 0),
        ]);
        let r = run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES);
        // Opus 4.8: 5 + 25 + 6.25 + 0.5; Sonnet 5.5: 2 x 2 + 10 x 0.1; gpt-4o: no price.
        let c = r.totals.cost;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(c.input, 9.0) && near(c.output, 25.0) && near(c.cache_write, 6.25) && near(c.cache_read, 1.5) && near(c.total, 41.75), "{c:?}");
        assert_eq!((r.totals.unpriced.input, r.totals.tokens.input), (3 * M, 6 * M), "unpriced tokens still count as tokens");
        assert_eq!(r.models.iter().find(|m| m.model == "gpt-4o").unwrap().cost, None);
        assert!(near(r.projects[0].cost, 41.75) && near(r.chats[0].cost, 41.75));
    }

    #[test]
    fn days_and_hours_follow_the_local_time_zone() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let at = |d: u32, h: u32, m: u32| tz.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap().timestamp_millis();
        let now = at(11, 15, 0);
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/c.jsonl", &[
            asst_at("a", at(10, 23, 50), "C:\\a", "claude-opus-4-8", 1, 0, 0, 0),
            asst_at("b", at(11, 0, 10), "C:\\a", "claude-sonnet-5-5", 10, 0, 0, 0),
            // The first minute of the 7-day window, and the last one before it.
            asst_at("c", at(5, 0, 0), "C:\\a", "claude-opus-4-8", 100, 0, 0, 0),
            asst_at("d", at(4, 23, 59), "C:\\a", "claude-opus-4-8", 1000, 0, 0, 0),
        ]);
        let mut store = Store::default();
        let scan = refresh(&mut store, tmp.path(), scan_from("7", now, &tz), MAX_FILE_BYTES, &quiet(2));
        let r = report(&store, &scan, &HashMap::new(), "7", now, &tz, MAX_FILE_BYTES);
        assert_eq!((r.start, r.days.len(), r.totals.tokens.work), (at(5, 0, 0), 7, 111));
        let work = |i: usize, f: &str| r.days[i].families.get(f).map_or(0, |a| a.work);
        assert_eq!((work(0, "opus"), work(5, "opus"), work(6, "sonnet"), work(6, "opus")), (100, 1, 10, 0));
        assert_eq!(r.days[6].start, at(11, 0, 0));
        assert_eq!((r.hours[23].work, r.hours[0].work), (1, 110));
        // "today" is the local day so far; all time starts on the first day with usage.
        let today = report(&store, &scan, &HashMap::new(), "today", now, &tz, MAX_FILE_BYTES);
        assert_eq!((today.days.len(), today.totals.tokens.work), (1, 10));
        let all = report(&store, &scan, &HashMap::new(), "all", now, &tz, MAX_FILE_BYTES);
        assert_eq!((all.start, all.days.len(), all.totals.tokens.work, all.previous.is_none()), (at(4, 0, 0), 8, 1111, true));
    }

    #[test]
    fn the_previous_period_covers_the_same_hours_one_span_earlier() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let at = |d: u32, h: u32| tz.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap().timestamp_millis();
        let now = at(11, 15);
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/c.jsonl", &[
            asst_at("today", at(11, 9), "C:\\a", "claude-opus-4-8", 200, 0, 0, 0),
            asst_at("yday-same", at(10, 9), "C:\\a", "claude-opus-4-8", 100, 0, 0, 0),
            // Later in the day than "now": outside yesterday's comparison span.
            asst_at("yday-later", at(10, 16), "C:\\a", "claude-opus-4-8", 1000, 0, 0, 0),
            asst_at("week-ago", at(4, 9), "C:\\a", "claude-opus-4-8", 7, 0, 0, 0),
        ]);
        let mut store = Store::default();
        let scan = refresh(&mut store, tmp.path(), scan_from("7", now, &tz), MAX_FILE_BYTES, &quiet(2));
        let today = report(&store, &scan, &HashMap::new(), "today", now, &tz, MAX_FILE_BYTES);
        let prev = today.previous.unwrap();
        assert_eq!((today.totals.tokens.work, prev.tokens.work), (200, 100));
        assert!((prev.cost - 100.0 * 5.0 / 1e6).abs() < 1e-12);
        // 7 days: Nov 5 00:00 - Nov 11 15:00 against Oct 29 00:00 - Nov 4 15:00.
        let week = report(&store, &scan, &HashMap::new(), "7", now, &tz, MAX_FILE_BYTES);
        assert_eq!((week.totals.tokens.work, week.previous.unwrap().tokens.work), (1300, 7));
    }

    #[test]
    fn subagent_transcripts_count_toward_the_parent_chat() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "p/c1.jsonl", &[asst("a", 0.1, "C:\\a", "claude-opus-4-8", 30, 0, 0, 0)]);
        write(tmp.path(), "p/c1/subagents/agent-1.jsonl", &[asst("b", 0.1, "C:\\a", "claude-opus-4-8", 10, 0, 0, 0)]);
        write(tmp.path(), "p/c1/subagents/nested/agent-2.jsonl", &[asst("c", 0.1, "C:\\a", "claude-haiku-4-5", 5, 0, 0, 0)]);
        // Other nested files belong to the chat but are not subagents.
        write(tmp.path(), "p/c1/other/x.jsonl", &[asst("d", 0.1, "C:\\a", "claude-opus-4-8", 1, 0, 0, 0)]);
        let r = run(tmp.path(), &HashMap::new(), "7", MAX_FILE_BYTES);
        assert_eq!((r.totals.tokens.work, r.totals.subagent_tokens.work), (46, 15));
        assert_eq!(r.chats.iter().map(|c| (c.cli_session_id.as_str(), c.tokens.work, c.messages)).collect::<Vec<_>>(), [("c1", 46, 4)]);
        assert!((r.totals.subagent_cost - (10.0 * 5.0 + 5.0 * 1.0) / 1e6).abs() < 1e-12);
    }

    #[test]
    fn csv_lists_every_row_and_never_overwrites() {
        crate::i18n::set_test_lang(crate::i18n::Lang::En);
        let tmp = tempfile::tempdir().unwrap();
        let (dir, out) = (tmp.path().join("projects"), tmp.path().join("downloads"));
        fs::create_dir_all(&out).unwrap();
        write(&dir, "p/c1.jsonl", &[asst("a", 0.1, "C:\\a, \"b\"", "claude-opus-4-8", 1_000_000, 0, 0, 0)]);
        write(&dir, "p/c2.jsonl", &[asst("b", 0.1, "C:\\x", "gpt-4o", 3, 4, 0, 0)]);
        let r = run(&dir, &HashMap::new(), "30", MAX_FILE_BYTES);
        let first = write_csv(&r, &out, "2026-11-11").unwrap();
        let second = write_csv(&r, &out, "2026-11-11").unwrap();
        assert!(first.ends_with("claude-harcama-30-gun-2026-11-11.csv"), "{first:?}");
        assert!(second.ends_with("claude-harcama-30-gun-2026-11-11 (2).csv"), "{second:?}");
        let text = fs::read_to_string(&first).unwrap();
        assert!(text.starts_with('\u{feff}'));
        let lines: Vec<_> = text.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "Section,Name,Folder,Tokens processed,Input,Output,Cache write,Cache read,API equivalent (USD),Messages,Last activity");
        assert_eq!(lines.len(), 1 + 2 + 2 + 2, "header, 2 projects, 2 chats, 2 models");
        assert!(lines[1].starts_with("Project,\"C:\\a, \"\"b\"\"\",\"C:\\a, \"\"b\"\"\",1000000,1000000,0,0,0,5.0000,1,"), "{}", lines[1]);
        assert!(lines.iter().any(|l| l.starts_with("Model,gpt-4o,,7,3,4,0,0,,1,")), "an unpriced model has an empty cost");
        assert_eq!(range_label("today"), "bugun");
        assert_eq!(range_label("all"), "tum-zamanlar");
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
            let scan = refresh(store, &dir, scan_from("7", now(), &Utc), MAX_FILE_BYTES, &quiet(threads));
            let r = report(store, &scan, &HashMap::new(), "7", now(), &Utc, MAX_FILE_BYTES);
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
        assert_eq!(warm.totals.tokens.total, exact_total(&dir, "7"));

        // All time at the owner's scale, built in memory: 5,000 chats, 1M messages over a year.
        let (mut big, mut files) = (Store::default(), Vec::new());
        for f in 0..5000u64 {
            let path = PathBuf::from(format!("C:/x/p{}/c{f}.jsonl", f % 40));
            let base = (now() - f as i64 * 365 * DAY_MS / 5000).div_euclid(SLOT_MS);
            let slots = (0..40u64)
                .map(|k| {
                    let s = base - k as i64;
                    let i = (0..5).map(|j| (f << 20 | k << 4 | j) & ID_MASK).collect();
                    Slot { s, m: "claude-opus-4-8".into(), c: format!("C:\\p{}", f % 40), t: [5, 300, 2_000, 40_000], n: 5, l: s * SLOT_MS, i }
                })
                .collect();
            big.files.insert(path.clone(), FileRec { slots, ..Default::default() });
            files.push(FileInfo { path, chat: format!("c{f}"), subagent: f % 7 == 0, mtime: now(), size: 1 });
        }
        let scan = Scan { files, failed: HashSet::new(), dirty: false };
        let t = Instant::now();
        let all = report(&big, &scan, &HashMap::new(), "all", now(), &Utc, MAX_FILE_BYTES);
        println!("all time, 1M messages: report {:>6.0} ms  ({} days, {} chats)", t.elapsed().as_secs_f64() * 1000.0, all.days.len(), all.chats.len());
        assert_eq!(all.totals.messages, 1_000_000);
        let t = Instant::now();
        save(&big, &studio).unwrap();
        let size = fs::metadata(studio.join(CACHE_FILE)).unwrap().len();
        println!("all time, 1M messages: save   {:>6.0} ms  ({:.1} MB cache)", t.elapsed().as_secs_f64() * 1000.0, size as f64 / 1e6);
        let t = Instant::now();
        let mut back = Store::default();
        load(&mut back, &studio);
        println!("all time, 1M messages: load   {:>6.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}
