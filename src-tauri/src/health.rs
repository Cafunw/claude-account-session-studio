//! Read-only health report: broken chat pointers and backups that fail SHA-256 verification.
//! Reads the pointer folders itself because `scan_all_sessions` silently skips unreadable files.
//! Never writes, moves or deletes anything.

use crate::accounts::{display_name, AccountProfile};
use crate::backup::{list_backups, verify_backup_integrity};
use crate::i18n::t;
use crate::sync_engine::index_available_cli_transcripts;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthItem {
    /// `"chat"` | `"backup"`
    pub kind: String,
    /// Account display name (chats) or backup reason (backups).
    pub owner: String,
    /// Chat title or file name, or backup id.
    pub subject: String,
    pub problem: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub checked_chats: usize,
    pub checked_backups: usize,
    pub chat_problems: usize,
    pub backup_problems: usize,
    pub items: Vec<HealthItem>,
}

pub fn build_report(
    data_root: &Path,
    projects_dir: &Path,
    accounts: &[AccountProfile],
    backup_storage_dir: &Path,
) -> HealthReport {
    let transcripts = index_available_cli_transcripts(projects_dir);
    let mut items = Vec::new();
    let mut checked_chats = 0;

    let mut acc_dirs: Vec<_> = fs::read_dir(data_root.join("claude-code-sessions"))
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    acc_dirs.sort();
    for acc_dir in acc_dirs {
        let uuid = acc_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let owner = accounts
            .iter()
            .find(|a| a.account_uuid == uuid)
            .map(display_name)
            .unwrap_or_else(|| {
                let id = uuid.chars().take(8).collect::<String>();
                t(format!("Hesap {id}"), format!("Account {id}"))
            });
        // A chat copied into two orgs of one account counts as listed twice.
        let mut seen_cli: HashSet<String> = HashSet::new();

        let mut files: Vec<_> = fs::read_dir(&acc_dir)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .flat_map(|org| fs::read_dir(org).map(|r| r.flatten().map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default())
            .collect();
        files.sort();
        for path in files {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !name.starts_with("local_") || !name.ends_with(".json") {
                continue;
            }
            checked_chats += 1;
            let mut push = |subject: &str, problem: &str| {
                items.push(HealthItem {
                    kind: "chat".into(),
                    owner: owner.clone(),
                    subject: subject.to_string(),
                    problem: problem.to_string(),
                })
            };
            let val = fs::read_to_string(&path).ok().and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            let Some(val) = val else {
                push(&name, t("Dosya okunamıyor veya bozuk (JSON geçersiz).", "File is unreadable or corrupt (invalid JSON)."));
                continue;
            };
            let title = val.get("title").and_then(|v| v.as_str()).unwrap_or(&name).to_string();
            let cli = val.get("cliSessionId").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            if val.get("transcriptUnavailable").and_then(|v| v.as_bool()).unwrap_or(false) {
                push(&title, t("Sohbet kaydı kullanılamıyor (transcriptUnavailable).", "Chat transcript is unavailable (transcriptUnavailable)."));
            } else if cli.is_some_and(|id| !transcripts.contains(id)) {
                push(&title, t("Sohbetin kayıt dosyası (.jsonl) diskte bulunamadı.", "The chat's transcript file (.jsonl) is missing from disk."));
            }
            if let Some(id) = cli {
                if !seen_cli.insert(id.to_string()) {
                    push(&title, t("Aynı sohbet bu hesapta birden fazla kez listelenmiş.", "The same chat is listed more than once in this account."));
                }
            }
        }
    }

    let backups = list_backups(backup_storage_dir);
    for b in &backups {
        if let Err(e) = verify_backup_integrity(backup_storage_dir, &b.id) {
            items.push(HealthItem {
                kind: "backup".into(),
                owner: b.reason.clone(),
                subject: b.id.clone(),
                problem: t(format!("Yedek doğrulaması başarısız: {e}"), format!("Backup verification failed: {e}")),
            });
        }
    }

    let backup_problems = items.iter().filter(|i| i.kind == "backup").count();
    HealthReport {
        checked_chats,
        checked_backups: backups.len(),
        chat_problems: items.len() - backup_problems,
        backup_problems,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{BackupFileEntry, BackupManifest};
    use crate::dpapi::sha256_bytes;

    #[test]
    fn reports_each_problem_kind_and_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("data");
        let org = root.join("claude-code-sessions").join("acc-1111111111").join("org-a");
        fs::create_dir_all(&org).unwrap();
        let projects = tmp.path().join("projects").join("p");
        fs::create_dir_all(&projects).unwrap();
        fs::write(projects.join("ok-cli.jsonl"), "{}").unwrap();

        fs::write(org.join("local_good.json"), r#"{"title":"Sağlam","cliSessionId":"ok-cli"}"#).unwrap();
        fs::write(org.join("local_broken.json"), "{ not json").unwrap();
        fs::write(org.join("local_unavail.json"), r#"{"title":"Kayıp","transcriptUnavailable":true}"#).unwrap();
        fs::write(org.join("local_nojsonl.json"), r#"{"title":"Dosyasız","cliSessionId":"gone"}"#).unwrap();
        fs::write(org.join("local_dup.json"), r#"{"title":"Çift","cliSessionId":"ok-cli"}"#).unwrap();

        let backups = tmp.path().join("backups");
        let bdir = backups.join("b1");
        fs::create_dir_all(&bdir).unwrap();
        let content = b"original";
        fs::write(bdir.join("a.json"), content).unwrap();
        let manifest = BackupManifest {
            id: "b1".into(),
            created_at: "2026-01-01T00:00:00+00:00".into(),
            reason: "test".into(),
            source_root: String::new(),
            backup_dir: bdir.to_string_lossy().into_owned(),
            total_files: 1,
            total_bytes: content.len() as u64,
            all_verified: true,
            files: vec![BackupFileEntry {
                relative_path: "a.json".into(),
                sha256: sha256_bytes(content),
                size_bytes: content.len() as u64,
                verified: true,
            }],
            kind: None,
        };
        fs::write(bdir.join("backup-manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();

        let clean = build_report(&root, tmp.path().join("projects").as_path(), &[], &backups);
        assert_eq!(clean.backup_problems, 0);

        fs::write(bdir.join("a.json"), b"tampered").unwrap();
        crate::i18n::set_test_lang(crate::i18n::Lang::Tr);
        let r = build_report(&root, tmp.path().join("projects").as_path(), &[], &backups);
        assert_eq!(r.checked_chats, 5);
        assert_eq!(r.checked_backups, 1);
        assert_eq!(r.backup_problems, 1);
        // broken, unavailable, missing jsonl, duplicate ("local_dup" sorts before "local_good")
        assert_eq!(r.chat_problems, 4, "{:?}", r.items);
        let has = |s: &str| r.items.iter().any(|i| i.problem.contains(s));
        assert!(has("bozuk") && has("transcriptUnavailable") && has("bulunamadı") && has("birden fazla"));
        assert!(has("SHA-256"));
        crate::i18n::set_test_lang(crate::i18n::Lang::En);
        let en = build_report(&root, tmp.path().join("projects").as_path(), &[], &backups);
        assert!(en.items.iter().any(|i| i.problem.contains("corrupt")) && en.items.iter().any(|i| i.problem.contains("more than once")));
        assert!(r.items.iter().all(|i| i.owner != "" ));
        assert_eq!(fs::read(bdir.join("a.json")).unwrap(), b"tampered"); // untouched
    }
}
