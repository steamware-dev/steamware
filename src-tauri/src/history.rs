//! Download history, kept across restarts.
//!
//! The queue in `downloads.rs` is in memory: it exists to run jobs, not to remember them. This module is
//! the record the Downloads page reads and the Home page counts its last-seven-days chart from, so it has
//! to survive a close. One JSON file in the profile, newest first, capped so it cannot grow without bound.
//!
//! Only what the UI shows is stored — the title, the app id, how the job ended, how big it was and when it
//! ran. Paths are deliberately left out: the file is a record of the download, not an index of the disk.

use crate::downloads::DownloadItem;
use crate::settings::app_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const MAX_ENTRIES: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub app_id: String,
    pub title: String,
    pub kind: String,
    /// done | failed | cancelled
    pub status: String,
    pub size: u64,
    pub started_at: i64,
    pub file_name: String,
}

fn path() -> PathBuf {
    app_dir().join("history.json")
}

pub fn load() -> Vec<HistoryEntry> {
    let Ok(text) = fs::read_to_string(path()) else {
        return Vec::new();
    };
    let mut entries: Vec<HistoryEntry> = serde_json::from_str(&text).unwrap_or_default();
    entries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    entries
}

/// Record one finished job. Same id twice replaces the earlier row, so a retry cannot show up as two.
pub fn record(item: &DownloadItem) {
    let entry = HistoryEntry {
        id: item.id.clone(),
        app_id: item.app_id.clone(),
        title: item.title.clone(),
        kind: item.kind.clone(),
        status: item.status.clone(),
        size: if item.total > 0 { item.total } else { item.received },
        started_at: item.started_at,
        file_name: item.file_name.clone(),
    };

    let mut entries = load();
    entries.retain(|existing| existing.id != entry.id);
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);

    let _ = fs::create_dir_all(app_dir());
    if let Ok(text) = serde_json::to_vec_pretty(&entries) {
        let _ = fs::write(path(), text);
    }
}

pub fn clear() {
    let _ = fs::remove_file(path());
}

/// Drop one row. The files on disk are not touched — this forgets the record, not the download.
pub fn remove(id: &str) {
    let mut entries = load();
    entries.retain(|entry| entry.id != id);
    if let Ok(text) = serde_json::to_vec_pretty(&entries) {
        let _ = fs::write(path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_newest_first() {
        let entries = vec![
            HistoryEntry {
                id: "b".into(),
                app_id: "2".into(),
                title: "Second".into(),
                kind: "manifest".into(),
                status: "done".into(),
                size: 10,
                started_at: 20,
                file_name: "2.zip".into(),
            },
            HistoryEntry {
                id: "a".into(),
                app_id: "1".into(),
                title: "First".into(),
                kind: "manifest".into(),
                status: "done".into(),
                size: 20,
                started_at: 10,
                file_name: "1.zip".into(),
            },
        ];
        let text = serde_json::to_string(&entries).unwrap();
        let mut read: Vec<HistoryEntry> = serde_json::from_str(&text).unwrap();
        read.sort_by(|x, y| y.started_at.cmp(&x.started_at));
        assert_eq!(read[0].id, "b");
        assert_eq!(read[1].id, "a");
    }
}
