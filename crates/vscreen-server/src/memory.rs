use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Screenshot History
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotEntry {
    pub timestamp_ms: u64,
    pub url: String,
    pub action_label: String,
    pub scroll_y: f64,
    pub viewport_height: u32,
    pub full_page: bool,
    #[serde(skip)]
    pub data: bytes::Bytes,
}

#[derive(Debug)]
pub struct ScreenshotHistory {
    entries: VecDeque<ScreenshotEntry>,
    max_entries: usize,
}

impl ScreenshotHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
        }
    }

    pub fn push(
        &mut self,
        data: bytes::Bytes,
        url: String,
        action_label: String,
        scroll_y: f64,
        viewport_height: u32,
        full_page: bool,
    ) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(ScreenshotEntry {
            timestamp_ms: now_ms(),
            url,
            action_label,
            scroll_y,
            viewport_height,
            full_page,
            data,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// List metadata for all entries (newest last).
    pub fn list(&self) -> Vec<ScreenshotEntryMeta> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| ScreenshotEntryMeta {
                index: i,
                timestamp_ms: e.timestamp_ms,
                url: e.url.clone(),
                action_label: e.action_label.clone(),
                scroll_y: e.scroll_y,
                full_page: e.full_page,
            })
            .collect()
    }

    /// Get a screenshot by index (0 = oldest in buffer).
    pub fn get(&self, index: usize) -> Option<&ScreenshotEntry> {
        self.entries.get(index)
    }

    /// Get the most recent screenshot.
    pub fn latest(&self) -> Option<&ScreenshotEntry> {
        self.entries.back()
    }

    /// Get a range of screenshots. `from` is inclusive, returns up to `count`.
    pub fn get_range(&self, from: usize, count: usize) -> Vec<&ScreenshotEntry> {
        self.entries.iter().skip(from).take(count).collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotEntryMeta {
    pub index: usize,
    pub timestamp_ms: u64,
    pub url: String,
    pub action_label: String,
    pub scroll_y: f64,
    pub full_page: bool,
}

// ---------------------------------------------------------------------------
// Action Log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub tool_name: String,
    pub params_summary: String,
    pub result_summary: String,
    pub url_before: String,
    pub url_after: String,
    pub screenshot_index: Option<usize>,
}

#[derive(Debug)]
pub struct ActionLog {
    entries: VecDeque<ActionEntry>,
    max_entries: usize,
    next_id: u64,
}

impl ActionLog {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
            next_id: 1,
        }
    }

    pub fn record(
        &mut self,
        tool_name: String,
        params_summary: String,
        result_summary: String,
        url_before: String,
        url_after: String,
        screenshot_index: Option<usize>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }

        self.entries.push_back(ActionEntry {
            id,
            timestamp_ms: now_ms(),
            tool_name,
            params_summary,
            result_summary,
            url_before,
            url_after,
            screenshot_index,
        });

        id
    }

    pub fn entries(&self) -> &VecDeque<ActionEntry> {
        &self.entries
    }

    pub fn last_n(&self, n: usize) -> Vec<&ActionEntry> {
        let skip = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(skip).collect()
    }

    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for entry in &self.entries {
            lines.push(format!(
                "#{}: {} — {}",
                entry.id, entry.tool_name, entry.result_summary
            ));
        }
        if lines.is_empty() {
            "No actions recorded yet.".into()
        } else {
            lines.join("\n")
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

// ---------------------------------------------------------------------------
// Console Buffer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub text: String,
}

#[derive(Debug)]
pub struct ConsoleBuffer {
    entries: VecDeque<ConsoleEntry>,
    max_entries: usize,
}

impl ConsoleBuffer {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
        }
    }

    pub fn push(&mut self, level: String, text: String) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(ConsoleEntry {
            timestamp_ms: now_ms(),
            level,
            text,
        });
    }

    pub fn entries(&self) -> &VecDeque<ConsoleEntry> {
        &self.entries
    }

    pub fn filter_by_level(&self, level: &str) -> Vec<&ConsoleEntry> {
        self.entries
            .iter()
            .filter(|e| e.level == level)
            .collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_history_push_and_evict() {
        let mut hist = ScreenshotHistory::new(3);
        for i in 0..5 {
            hist.push(
                bytes::Bytes::from(vec![i as u8; 10]),
                format!("https://example.com/{i}"),
                format!("action_{i}"),
                0.0,
                940,
                false,
            );
        }
        assert_eq!(hist.len(), 3);
        assert_eq!(hist.get(0).unwrap().url, "https://example.com/2");
        assert_eq!(hist.latest().unwrap().url, "https://example.com/4");
    }

    #[test]
    fn screenshot_history_list_metadata() {
        let mut hist = ScreenshotHistory::new(10);
        hist.push(bytes::Bytes::new(), "url1".into(), "click".into(), 0.0, 940, false);
        hist.push(bytes::Bytes::new(), "url2".into(), "nav".into(), 100.0, 940, true);
        let meta = hist.list();
        assert_eq!(meta.len(), 2);
        assert_eq!(meta[0].index, 0);
        assert_eq!(meta[1].action_label, "nav");
        assert!(meta[1].full_page);
    }

    #[test]
    fn screenshot_history_get_range() {
        let mut hist = ScreenshotHistory::new(10);
        for i in 0..5 {
            hist.push(bytes::Bytes::new(), format!("url{i}"), "a".into(), 0.0, 940, false);
        }
        let range = hist.get_range(2, 2);
        assert_eq!(range.len(), 2);
        assert_eq!(range[0].url, "url2");
        assert_eq!(range[1].url, "url3");
    }

    #[test]
    fn screenshot_history_clear() {
        let mut hist = ScreenshotHistory::new(10);
        hist.push(bytes::Bytes::new(), "url".into(), "a".into(), 0.0, 940, false);
        hist.clear();
        assert!(hist.is_empty());
    }

    #[test]
    fn action_log_records_and_evicts() {
        let mut log = ActionLog::new(3);
        for i in 0..5 {
            log.record(
                format!("tool_{i}"),
                "params".into(),
                format!("result_{i}"),
                "before".into(),
                "after".into(),
                None,
            );
        }
        assert_eq!(log.len(), 3);
        let entries: Vec<_> = log.entries().iter().collect();
        assert_eq!(entries[0].id, 3);
        assert_eq!(entries[2].id, 5);
    }

    #[test]
    fn action_log_last_n() {
        let mut log = ActionLog::new(100);
        for i in 0..10 {
            log.record(format!("t{i}"), "p".into(), format!("r{i}"), "b".into(), "a".into(), None);
        }
        let last3 = log.last_n(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].tool_name, "t7");
    }

    #[test]
    fn action_log_summary() {
        let mut log = ActionLog::new(100);
        log.record("click".into(), "x=5".into(), "Clicked".into(), "u".into(), "u".into(), None);
        let s = log.summary();
        assert!(s.contains("click"));
        assert!(s.contains("Clicked"));
    }

    #[test]
    fn action_log_empty_summary() {
        let log = ActionLog::new(100);
        assert_eq!(log.summary(), "No actions recorded yet.");
    }

    #[test]
    fn console_buffer_push_and_evict() {
        let mut buf = ConsoleBuffer::new(3);
        for i in 0..5 {
            buf.push("log".into(), format!("msg {i}"));
        }
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn console_buffer_filter() {
        let mut buf = ConsoleBuffer::new(100);
        buf.push("log".into(), "normal".into());
        buf.push("error".into(), "bad".into());
        buf.push("warn".into(), "careful".into());
        buf.push("error".into(), "very bad".into());
        let errors = buf.filter_by_level("error");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn console_buffer_clear() {
        let mut buf = ConsoleBuffer::new(100);
        buf.push("log".into(), "msg".into());
        buf.clear();
        assert!(buf.is_empty());
    }
}
