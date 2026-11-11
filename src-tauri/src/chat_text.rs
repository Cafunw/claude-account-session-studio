//! Chat-content search and Markdown export. Read-only over Claude Code transcripts
//! (`~/.claude/projects/<project>/<cliSessionId>.jsonl`); the only thing written is a new export file.

use crate::sync_engine::{scan_all_sessions, SessionRecord};
use crate::i18n;
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use tauri::State;

const MAX_HITS: usize = 50;
const SNIPPET_SIDE: usize = 60;
const MAX_FILE_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub cli_session_id: String,
    pub title: String,
    pub account_uuid: String,
    pub snippet: String,
    /// "user" or "assistant".
    pub matched_role: String,
    pub last_activity_at: i64,
}

enum Part {
    Text(String),
    Tool(String),
}

/// One transcript line as (is_user, parts). Lines that are not user/assistant messages, or that
/// carry only tool results, give `None`.
fn parse_line(line: &str) -> Option<(bool, Vec<Part>)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let is_user = match v.get("type")?.as_str()? {
        "user" => true,
        "assistant" => false,
        _ => return None,
    };
    let parts = match v.get("message")?.get("content")? {
        Value::String(s) => vec![Part::Text(s.clone())],
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b.get("type")?.as_str()? {
                "text" => Some(Part::Text(b.get("text")?.as_str()?.to_string())),
                "tool_use" => Some(Part::Tool(b.get("name")?.as_str()?.to_string())),
                _ => None,
            })
            .collect(),
        _ => return None,
    };
    let parts: Vec<Part> = parts
        .into_iter()
        .filter(|p| !matches!(p, Part::Text(t) if t.trim().is_empty()))
        .collect();
    (!parts.is_empty()).then_some((is_user, parts))
}

/// Calls `f` for every user/assistant message; `f` returns false to stop early.
/// Files over the size cap are an error, never partially read.
fn for_each_message(path: &Path, mut f: impl FnMut(bool, Vec<Part>) -> bool) -> Result<(), String> {
    let file = File::open(path).map_err(|e| i18n::t(format!("Sohbet dosyası açılamadı: {e}"), format!("Couldn't open the chat file: {e}")))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
        return Err(i18n::t("Sohbet dosyası çok büyük (200 MB üstü).", "The chat file is too large (over 200 MB).").into());
    }
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(|e| e.to_string())? == 0 {
            return Ok(());
        }
        if let Some((is_user, parts)) = parse_line(&String::from_utf8_lossy(&buf)) {
            if !f(is_user, parts) {
                return Ok(());
            }
        }
    }
}

/// Case folding that keeps one char per char (so indexes line up) and treats Turkish
/// İ / I / ı as the same letter as i.
fn fold(s: &str) -> Vec<char> {
    s.chars()
        .map(|c| match c {
            'İ' | 'I' | 'ı' => 'i',
            _ => c.to_lowercase().next().unwrap_or(c),
        })
        .collect()
}

/// About 60 chars either side of the first match, whitespace collapsed.
fn snippet_for(text: &str, query: &[char]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let folded = fold(text);
    let at = folded.windows(query.len()).position(|w| w == query)?;
    let from = at.saturating_sub(SNIPPET_SIDE);
    let to = (at + query.len() + SNIPPET_SIDE).min(chars.len());
    let body = chars[from..to].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    Some(format!("{}{}{}", if from > 0 { "…" } else { "" }, body, if to < chars.len() { "…" } else { "" }))
}

/// All `<cliSessionId>.jsonl` files under the projects dir, by id.
fn index_transcripts(projects_dir: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for proj in fs::read_dir(projects_dir).into_iter().flatten().flatten() {
        for f in fs::read_dir(proj.path()).into_iter().flatten().flatten() {
            let path = f.path();
            if let Some(stem) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".jsonl")) {
                map.entry(stem.to_string()).or_insert_with(|| path.clone());
            }
        }
    }
    map
}

fn search_in(projects_dir: &Path, sessions: &[SessionRecord], query: &str) -> Vec<SearchHit> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let index = index_transcripts(projects_dir);
    let mut hits = Vec::new();
    for s in sessions {
        let Some(id) = &s.cli_session_id else { continue };
        let Some(path) = index.get(id) else { continue };
        let mut found = None;
        // An unreadable or oversized transcript is skipped, it must not fail the whole search.
        let _ = for_each_message(path, |is_user, parts| {
            for p in parts {
                if let Part::Text(t) = p {
                    if let Some(sn) = snippet_for(&t, &q) {
                        found = Some((sn, if is_user { "user" } else { "assistant" }));
                        return false;
                    }
                }
            }
            true
        });
        if let Some((snippet, role)) = found {
            hits.push(SearchHit {
                cli_session_id: id.clone(),
                title: s.title.clone(),
                account_uuid: s.account_uuid.clone(),
                snippet,
                matched_role: role.into(),
                last_activity_at: s.last_activity_at,
            });
            if hits.len() >= MAX_HITS {
                break;
            }
        }
    }
    hits
}

fn safe_file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '-' } else { c })
        .take(80)
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() { i18n::t("Sohbet", "Chat").into() } else { cleaned.into() }
}

fn day_of(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now)
        .format("%Y-%m-%d")
        .to_string()
}

/// Creates a new file in `dir`; an existing name gets " (2)", " (3)"... so nothing is overwritten.
fn create_unique(dir: &Path, stem: &str) -> Result<(File, PathBuf), String> {
    for n in 1u32.. {
        let name = if n == 1 { format!("{stem}.md") } else { format!("{stem} ({n}).md") };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((f, path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(write_failed(e)),
        }
    }
    unreachable!()
}

fn write_failed(e: std::io::Error) -> String {
    i18n::t(format!("Dosya yazılamadı: {e}"), format!("Couldn't write the file: {e}"))
}

fn export_markdown(transcript: &Path, session: &SessionRecord, dir: &Path) -> Result<PathBuf, String> {
    let (folder, date, you, tool) = i18n::t(("Klasör", "Tarih", "Sen", "Araç"), ("Folder", "Date", "You", "Tool"));
    let mut md = format!("# {}\n\n", session.title);
    if let Some(cwd) = session.cwd.as_deref().or(session.origin_cwd.as_deref()) {
        md.push_str(&format!("**{folder}:** {cwd}  \n"));
    }
    let day = day_of(session.last_activity_at);
    md.push_str(&format!("**{date}:** {day}\n"));
    for_each_message(transcript, |is_user, parts| {
        md.push_str(&format!("\n---\n\n**{}:**\n", if is_user { you } else { "Claude" }));
        for p in parts {
            match p {
                Part::Text(t) => md.push_str(&format!("\n{}\n", t.trim())),
                Part::Tool(name) => md.push_str(&format!("\n_{tool}: {name}_\n")),
            }
        }
        true
    })?;
    fs::create_dir_all(dir).map_err(|e| i18n::t(format!("Klasör oluşturulamadı: {e}"), format!("Couldn't create the folder: {e}")))?;
    let (mut file, path) = create_unique(dir, &format!("{} ({day})", safe_file_stem(&session.title)))?;
    file.write_all(md.as_bytes()).map_err(write_failed)?;
    Ok(path)
}

fn export_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default"))
        .join("Downloads")
        .join("Claude Sohbetleri") // fixed name: `reveal_in_explorer` only opens files inside it
}

/// Canonical path of `path` if it is inside `dir`, else an error.
fn inside_dir(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    let canon = path.canonicalize().map_err(|_| i18n::t("Dosya bulunamadı.", "File not found.").to_string())?;
    let base = dir.canonicalize().map_err(|_| i18n::t("Dışa aktarma klasörü bulunamadı.", "Export folder not found.").to_string())?;
    if canon.starts_with(&base) {
        Ok(canon)
    } else {
        Err(i18n::t("Yalnızca dışa aktarılan dosyalar gösterilebilir.", "Only exported files can be shown.").into())
    }
}

/// `~/.claude/projects`.
pub(crate) fn cli_projects_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default"))
        .join(".claude")
        .join("projects")
}

/// The chats the app lists: one per CLI session id, newest first, with a transcript id.
pub(crate) fn listed_chats(state: &AppState) -> (PathBuf, Vec<SessionRecord>) {
    let projects = cli_projects_dir();
    let data_root = crate::paths::discover_real_roots()
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| state.studio_dir.join("fallback-data"));
    let accounts = state.config.lock().unwrap().accounts.clone();
    let mut sessions = scan_all_sessions(&data_root, &projects, &accounts);
    sessions.sort_by(|a, b| b.last_activity_at.cmp(&a.last_activity_at));
    let mut seen = HashSet::new();
    sessions.retain(|s| s.cli_session_id.as_ref().is_some_and(|id| seen.insert(id.clone())));
    (projects, sessions)
}

#[tauri::command(async)]
pub fn search_chats(query: String, state: State<'_, AppState>) -> Vec<SearchHit> {
    let (projects, sessions) = listed_chats(&state);
    search_in(&projects, &sessions, &query)
}

#[tauri::command(async)]
pub fn export_chat_markdown(cli_session_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (projects, sessions) = listed_chats(&state);
    let session = sessions
        .iter()
        .find(|s| s.cli_session_id.as_deref() == Some(cli_session_id.as_str()))
        .ok_or_else(|| i18n::t("Sohbet bulunamadı.", "Chat not found.").to_string())?;
    let transcript = index_transcripts(&projects)
        .remove(&cli_session_id)
        .ok_or_else(|| i18n::t("Bu sohbetin içeriği diskte yok.", "This chat's content is not on disk.").to_string())?;
    Ok(export_markdown(&transcript, session, &export_dir())?.to_string_lossy().into_owned())
}

#[tauri::command(async)]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    let canon = inside_dir(Path::new(&path), &export_dir())?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let shown = canon.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        // raw_arg: explorer wants /select,"C:\a b\c.md" with the quotes around the path only.
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{shown}\""))
            .spawn()
            .map_err(|e| i18n::t(format!("Gezgin açılamadı: {e}"), format!("Couldn't open File Explorer: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = canon;
        Err(i18n::t("Yalnızca Windows'ta desteklenir.", "Only supported on Windows.").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str, title: &str) -> SessionRecord {
        SessionRecord {
            session_id: format!("local_{id}"),
            file_name: String::new(),
            file_path: String::new(),
            account_uuid: "acc-1".into(),
            account_label: String::new(),
            org_uuid: String::new(),
            cli_session_id: Some(id.into()),
            prior_cli_session_ids: Vec::new(),
            cwd: Some("C:\\work\\demo".into()),
            origin_cwd: None,
            title: title.into(),
            created_at: 0,
            last_activity_at: 1_700_000_000_000,
            completed_turns: 0,
            model: String::new(),
            is_archived: false,
            is_starred: false,
            transcript_unavailable: false,
            transcript_exists_on_disk: true,
            error_notice: None,
            spawned_from: None,
            group_id: None,
        }
    }

    fn write_transcript(projects: &Path, id: &str, lines: &[Value]) -> PathBuf {
        let dir = projects.join("proj");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{id}.jsonl"));
        fs::write(&path, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
        path
    }

    fn user(text: &str) -> Value {
        json!({"type": "user", "message": {"role": "user", "content": text}})
    }

    fn assistant(blocks: Value) -> Value {
        json!({"type": "assistant", "message": {"role": "assistant", "content": blocks}})
    }

    #[test]
    fn extracts_text_from_strings_and_blocks_and_skips_tool_payloads() {
        let (is_user, parts) = parse_line(&user("merhaba").to_string()).unwrap();
        assert!(is_user && matches!(&parts[..], [Part::Text(t)] if t == "merhaba"));

        let line = assistant(json!([
            {"type": "thinking", "thinking": "gizli"},
            {"type": "text", "text": "cevap"},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "gizli-yol"}}
        ]));
        let (is_user, parts) = parse_line(&line.to_string()).unwrap();
        assert!(!is_user);
        assert!(matches!(&parts[..], [Part::Text(t), Part::Tool(n)] if t == "cevap" && n == "Read"));

        let result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "çıktı"}]}});
        assert!(parse_line(&result.to_string()).is_none());
        assert!(parse_line(r#"{"type":"summary","summary":"x"}"#).is_none());
        assert!(parse_line("not json").is_none());
    }

    #[test]
    fn search_is_case_and_turkish_insensitive_with_snippet_and_role() {
        let tmp = tempfile::tempdir().unwrap();
        let long = format!("{} İSTANBUL yolculuğu {}", "a".repeat(100), "b".repeat(100));
        write_transcript(
            tmp.path(),
            "c1",
            &[user("selam"), assistant(json!([{"type": "tool_use", "name": "istanbul", "input": {}}, {"type": "text", "text": long}]))],
        );
        let hits = search_in(tmp.path(), &[session("c1", "Gezi")], "istanbul");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_role, "assistant");
        assert!(hits[0].snippet.starts_with('…') && hits[0].snippet.ends_with('…'));
        assert!(hits[0].snippet.contains("İSTANBUL yolculuğu"));
        assert_eq!(hits[0].snippet.chars().count(), 2 + 60 + 8 + 60);
        // Tool payloads alone never match.
        write_transcript(tmp.path(), "c2", &[assistant(json!([{"type": "tool_use", "name": "Read", "input": {"p": "zebra"}}]))]);
        assert!(search_in(tmp.path(), &[session("c2", "x")], "zebra").is_empty());
        assert!(search_in(tmp.path(), &[session("c1", "x")], "   ").is_empty());
    }

    #[test]
    fn search_gives_one_hit_per_chat_and_at_most_fifty() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sessions = Vec::new();
        for i in 0..60 {
            let id = format!("chat-{i}");
            write_transcript(tmp.path(), &id, &[user("anahtar bir"), user("anahtar iki")]);
            sessions.push(session(&id, "t"));
        }
        // Listed but with no transcript on disk: skipped.
        sessions.push(session("missing", "t"));
        let hits = search_in(tmp.path(), &sessions, "ANAHTAR");
        assert_eq!(hits.len(), 50);
        assert_eq!(hits.iter().map(|h| &h.cli_session_id).collect::<HashSet<_>>().len(), 50);
    }

    #[test]
    fn markdown_has_header_messages_tool_lines_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("projects");
        let t = write_transcript(
            &projects,
            "c1",
            &[
                user("soru?"),
                assistant(json!([{"type": "text", "text": "cevap"}, {"type": "tool_use", "name": "Bash", "input": {"command": "gizli"}}])),
                json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "gizli çıktı"}]}}),
            ],
        );
        let out = tmp.path().join("out");
        let s = session("c1", "Plan: a/b?");
        i18n::set_test_lang(i18n::Lang::Tr);
        let p1 = export_markdown(&t, &s, &out).unwrap();
        let name = p1.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("Plan- a-b- (20") && name.ends_with(").md"), "{name}");
        let md = fs::read_to_string(&p1).unwrap();
        assert!(md.starts_with("# Plan: a/b?\n"));
        assert!(md.contains("**Klasör:** C:\\work\\demo") && md.contains("**Tarih:** 20"));
        assert!(md.contains("**Sen:**\n\nsoru?") && md.contains("**Claude:**\n\ncevap"));
        assert!(md.contains("_Araç: Bash_") && !md.contains("gizli"));

        let p2 = export_markdown(&t, &s, &out).unwrap();
        let p3 = export_markdown(&t, &s, &out).unwrap();
        assert!(p2.to_string_lossy().ends_with(" (2).md") && p3.to_string_lossy().ends_with(" (3).md"));
        assert_eq!(fs::read_to_string(&p1).unwrap(), md);

        i18n::set_test_lang(i18n::Lang::En);
        let en = fs::read_to_string(export_markdown(&t, &s, &out).unwrap()).unwrap();
        assert!(en.contains("**Folder:** C:\\work\\demo") && en.contains("**Date:** 20"));
        assert!(en.contains("**You:**\n\nsoru?") && en.contains("**Claude:**\n\ncevap") && en.contains("_Tool: Bash_"));
    }

    #[test]
    fn oversized_or_unknown_files_are_not_read() {
        assert!(for_each_message(Path::new("yok.jsonl"), |_, _| true).is_err());
        i18n::set_test_lang(i18n::Lang::Tr);
        assert_eq!(safe_file_stem("  ... "), "Sohbet");
        i18n::set_test_lang(i18n::Lang::En);
        assert_eq!(safe_file_stem("  ... "), "Chat");
    }

    #[test]
    fn reveal_refuses_paths_outside_the_export_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("exports");
        fs::create_dir_all(&dir).unwrap();
        let inside = dir.join("a.md");
        let outside = tmp.path().join("b.md");
        fs::write(&inside, "x").unwrap();
        fs::write(&outside, "x").unwrap();
        assert!(inside_dir(&inside, &dir).is_ok());
        assert!(inside_dir(&outside, &dir).is_err());
        // `..` that climbs out is resolved before the check.
        assert!(inside_dir(&dir.join("..").join("b.md"), &dir).is_err());
        assert!(inside_dir(&dir.join("yok.md"), &dir).is_err());
    }
}
