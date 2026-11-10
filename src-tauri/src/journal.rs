//! Read-only access to the operation journal for the "İşlem günlüğü" page.
use crate::backup::{read_journal, JournalEntry};
use crate::AppState;
use tauri::State;

/// Newest first.
#[tauri::command(async)]
pub fn get_journal(limit: usize, state: State<'_, AppState>) -> Vec<JournalEntry> {
    read_journal(&state.backup_storage_dir, limit)
}
