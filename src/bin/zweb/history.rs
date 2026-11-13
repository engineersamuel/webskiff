//! Visited pages, ranked by frecency, for completion in the Open prompts.
//!
//! Kept in memory; saved as TSV (url, title, visits, last visit in Unix
//! seconds) by a background thread, so no key press waits for the disk.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CAP: usize = 2000;
/// Matches kept for completion.
const LIMIT: usize = 9;
const DAY: u64 = 86_400;
/// (age below, in days; weight). Older pages weigh 10.
const WEIGHTS: [(u64, u64); 4] = [(4, 100), (14, 70), (31, 50), (90, 30)];
pub const SAVE_DELAY: Duration = Duration::from_secs(5);

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn eligible(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && !url.contains(['\t', '\n', '\r'])
}

#[derive(Clone)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Last visit, Unix seconds.
    pub last: u64,
    url_lc: String,
    title_lc: String,
}

impl Entry {
    fn new(url: String, title: String, visits: u32, last: u64) -> Self {
        let mut entry = Self { url_lc: url.to_lowercase(), url, title: String::new(), title_lc: String::new(), visits, last };
        entry.set_title(&title);
        entry
    }

    fn parse(line: &str) -> Option<Self> {
        let mut f = line.splitn(4, '\t');
        let (url, title) = (f.next()?, f.next()?);
        let (visits, last) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
        eligible(url).then(|| Self::new(url.into(), title.into(), visits, last))
    }

    fn set_title(&mut self, title: &str) {
        self.title = title.replace(['\t', '\n', '\r'], " ");
        self.title_lc = self.title.to_lowercase();
    }

    fn frecency(&self, now: u64) -> u64 {
        let age = now.saturating_sub(self.last) / DAY;
        let weight = WEIGHTS.iter().find(|(below, _)| age < *below).map_or(10, |(_, w)| *w);
        self.visits as u64 * weight
    }

    fn matches(&self, terms: &[&str]) -> bool {
        terms.iter().all(|t| self.url_lc.contains(t) || self.title_lc.contains(t))
    }
}

#[derive(Clone, Default)]
pub struct History {
    entries: Vec<Entry>,
}

impl History {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            text => text?,
        };
        let mut entries: Vec<Entry> = text.lines().filter_map(Entry::parse).collect();
        if entries.len() > CAP {
            let now = now();
            entries.select_nth_unstable_by_key(CAP, |e| std::cmp::Reverse(e.frecency(now)));
            entries.truncate(CAP);
        }
        Ok(Self { entries })
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn get(&self, url: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.url == url)
    }

    /// Counts one visit at `now`. Returns whether the history changed.
    pub fn visit(&mut self, url: &str, now: u64) -> bool {
        if !eligible(url) {
            return false;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.url == url) {
            entry.visits += 1;
            entry.last = entry.last.max(now);
            return true;
        }
        if self.entries.len() >= CAP {
            let lowest = self.entries.iter().enumerate().min_by_key(|(_, e)| e.frecency(now)).map(|(i, _)| i);
            self.entries.swap_remove(lowest.unwrap_or(0));
        }
        self.entries.push(Entry::new(url.into(), String::new(), 1, now));
        true
    }

    /// Returns whether the history changed.
    pub fn set_title(&mut self, url: &str, title: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.url == url) {
            Some(entry) if !title.trim().is_empty() => {
                entry.set_title(title);
                true
            }
            _ => false,
        }
    }

    /// The best `LIMIT` entries that hold every term of `query`, best first.
    pub fn top(&self, query: &str, now: u64) -> Vec<&Entry> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut best = [(0u64, 0usize); LIMIT];
        let mut n = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.matches(&terms) {
                continue;
            }
            let score = entry.frecency(now);
            if n == LIMIT && score <= best[LIMIT - 1].0 {
                continue;
            }
            let mut at = n.min(LIMIT - 1);
            n = (n + 1).min(LIMIT);
            while at > 0 && best[at - 1].0 < score {
                best[at] = best[at - 1];
                at -= 1;
            }
            best[at] = (score, i);
        }
        best[..n].iter().map(|&(_, i)| &self.entries[i]).collect()
    }

    /// Writes a temporary file readable by the user only, then renames it over `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let text: String =
            self.entries.iter().map(|e| format!("{}\t{}\t{}\t{}\n", e.url, e.title, e.visits, e.last)).collect();
        let tmp = path.with_extension("tsv.tmp");
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(text.as_bytes())?;
        std::fs::rename(&tmp, path)
    }
}

/// Saves snapshots on its own thread, `SAVE_DELAY` after the last change.
pub struct Saver {
    to_thread: Option<Sender<History>>,
    thread: Option<JoinHandle<()>>,
    changed_at: Option<Instant>,
}

impl Saver {
    pub fn spawn(path: PathBuf) -> Self {
        let (to_thread, from_app) = channel::<History>();
        let thread = std::thread::spawn(move || {
            for snapshot in from_app {
                if let Err(error) = snapshot.save(&path) {
                    eprintln!("webskiff: cannot save history {}: {error}", path.display());
                }
            }
        });
        Self { to_thread: Some(to_thread), thread: Some(thread), changed_at: None }
    }

    pub fn changed(&mut self, at: Instant) {
        self.changed_at = Some(at);
    }

    /// Sends a snapshot when the history has been quiet for `SAVE_DELAY`.
    pub fn tick(&mut self, history: &History, now: Instant) -> bool {
        if self.changed_at.is_none_or(|at| now < at + SAVE_DELAY) {
            return false;
        }
        self.send(history);
        true
    }

    fn send(&mut self, history: &History) {
        self.changed_at = None;
        if let Some(to_thread) = &self.to_thread {
            let _ = to_thread.send(history.clone());
        }
    }

    /// Saves unsaved changes and waits for the thread to finish writing.
    pub fn finish(&mut self, history: &History) {
        if self.changed_at.is_some() {
            self.send(history);
        }
        self.to_thread = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One Tab/Shift-Tab session: the matches for the typed text, and the one shown.
pub struct Completion {
    urls: Vec<String>,
    at: Option<usize>,
}

impl Completion {
    pub fn new(history: &History, typed: &str, now: u64) -> Self {
        Self { urls: history.top(typed, now).iter().map(|e| e.url.clone()).collect(), at: None }
    }

    pub fn step(&mut self, back: bool) -> Option<&str> {
        let n = self.urls.len();
        if n == 0 {
            return None;
        }
        let at = match (self.at, back) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.at = Some(at);
        Some(&self.urls[at])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    const NOW: u64 = 1_000 * DAY;

    fn urls(found: &[&Entry]) -> Vec<String> {
        found.iter().map(|e| e.url.clone()).collect()
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("webskiff-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn history_records_visits_and_repeat_visits() {
        let mut history = History::default();
        history.visit("https://example.com/", NOW - DAY);
        history.visit("https://example.com/", NOW);
        history.visit("https://other.org/", NOW);
        let entry = history.get("https://example.com/").unwrap();
        assert_eq!(entry.visits, 2);
        assert_eq!(entry.last, NOW);
        assert_eq!(history.get("https://other.org/").unwrap().visits, 1);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn history_updates_titles_without_tabs_or_newlines() {
        let mut history = History::default();
        history.visit("https://example.com/", NOW);
        history.set_title("https://example.com/", "Example\tDomain\nHome");
        assert_eq!(history.get("https://example.com/").unwrap().title, "Example Domain Home");
        history.set_title("https://unknown.org/", "ignored");
        assert!(history.get("https://unknown.org/").is_none());
    }

    #[test]
    fn history_matches_every_term_case_insensitively_in_url_or_title() {
        let mut history = History::default();
        for (url, title) in [
            ("https://github.com/rust-lang/rust", "The Rust Programming Language"),
            ("https://github.com/torvalds/linux", "Linux kernel source tree"),
            ("https://www.rust-lang.org/learn", "Learn Rust"),
        ] {
            history.visit(url, NOW);
            history.set_title(url, title);
        }
        assert_eq!(urls(&history.top("GITHUB rust", NOW)), ["https://github.com/rust-lang/rust"]);
        assert_eq!(urls(&history.top("kernel  GitHub", NOW)), ["https://github.com/torvalds/linux"]);
        assert_eq!(history.top("rust", NOW).len(), 2);
        assert!(history.top("github missing", NOW).is_empty());
    }

    #[test]
    fn history_ranks_by_frecency_at_a_fixed_time() {
        let mut history = History::default();
        for _ in 0..5 {
            history.visit("https://old.example/", NOW - 60 * DAY);
        }
        history.visit("https://once.example/", NOW);
        for _ in 0..5 {
            history.visit("https://often.example/", NOW);
        }
        let found = urls(&history.top("example", NOW));
        assert_eq!(found[0], "https://often.example/");
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn history_keeps_at_most_2000_entries_and_evicts_the_lowest_frecency() {
        let mut history = History::default();
        history.visit("https://stale.example/", NOW - 200 * DAY);
        for i in 0..1999 {
            history.visit(&format!("https://site{i}.example/"), NOW);
        }
        assert_eq!(history.len(), 2000);
        history.visit("https://new.example/", NOW);
        history.visit("https://newer.example/", NOW);
        assert_eq!(history.len(), 2000);
        assert!(history.get("https://stale.example/").is_none());
        assert!(history.get("https://newer.example/").is_some());
        assert!(history.get("https://new.example/").is_some());
    }

    #[test]
    fn history_records_only_http_and_https() {
        let mut history = History::default();
        for url in ["file:///etc/passwd", "about:blank", "data:text/html,hi", "chrome://settings/"] {
            history.visit(url, NOW);
            assert!(history.get(url).is_none(), "{url}");
        }
        history.visit("http://plain.example/", NOW);
        history.visit("https://secure.example/", NOW);
        assert!(history.get("http://plain.example/").is_some());
        assert!(history.get("https://secure.example/").is_some());
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn history_save_and_load_round_trip() {
        let dir = temp_dir("round-trip");
        let path = dir.join("history.tsv");
        let mut history = History::default();
        history.visit("https://example.com/a", NOW - 3 * DAY);
        history.visit("https://example.com/a", NOW - DAY);
        history.set_title("https://example.com/a", "Page A");
        history.visit("https://example.com/b", NOW);
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
        let a = loaded.get("https://example.com/a").unwrap();
        assert_eq!((a.title.as_str(), a.visits, a.last), ("Page A", 2, NOW - DAY));
        let b = loaded.get("https://example.com/b").unwrap();
        assert_eq!((b.title.as_str(), b.visits, b.last), ("", 1, NOW));
        assert_eq!(urls(&loaded.top("PAGE a", NOW)), ["https://example.com/a"]);
    }

    #[test]
    fn history_load_bounds_oversized_file_to_2000_highest_ranked_entries() {
        let dir = temp_dir("oversized");
        let path = dir.join("history.tsv");
        let mut raw = String::from("https://stale.example/\tStale\t1\t0\n");
        let recent = now();
        for i in 0..2000 {
            raw.push_str(&format!("https://site{i}.example/\tSite {i}\t2\t{recent}\n"));
        }
        std::fs::write(&path, raw).unwrap();
        let mut history = History::load(&path).unwrap();
        assert_eq!(history.len(), 2000);
        assert!(history.get("https://stale.example/").is_none());
        history.visit("https://fresh.example/", recent);
        history.save(&path).unwrap();
        let saved = History::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(saved.len(), 2000);
        assert!(saved.get("https://fresh.example/").is_some());
    }

    #[test]
    fn history_load_of_a_missing_file_is_empty() {
        let dir = temp_dir("missing");
        let loaded = History::load(&dir.join("history.tsv")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn history_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join("history.tsv");
        let mut history = History::default();
        history.visit("https://example.com/", NOW);
        history.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn history_saver_waits_for_quiet_and_saves_on_finish() {
        let dir = temp_dir("saver");
        let path = dir.join("history.tsv");
        let start = std::time::Instant::now();
        let mut saver = Saver::spawn(path.clone());
        let mut history = History::default();
        history.visit("https://first.example/", NOW);
        saver.changed(start);
        assert!(!saver.tick(&history, start + SAVE_DELAY / 2));
        assert!(saver.tick(&history, start + SAVE_DELAY));
        assert!(!saver.tick(&history, start + 2 * SAVE_DELAY));
        history.visit("https://second.example/", NOW);
        saver.changed(start);
        saver.finish(&history);
        let loaded = History::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn history_saver_retries_failed_background_save_on_finish_without_later_changes() {
        let dir = temp_dir("saver-retry");
        let path = dir.join("missing").join("history.tsv");
        let start = Instant::now();
        let mut saver = Saver::spawn(path.clone());
        let mut history = History::default();
        for (url, at) in [
            ("https://first.example/", NOW - DAY),
            ("https://first.example/", NOW),
            ("https://second.example/", NOW - DAY),
        ] {
            assert!(history.visit(url, at));
            saver.changed(start);
        }
        assert!(saver.tick(&history, start + SAVE_DELAY));
        // Drain and join the writer before recovery, so the queued write must fail.
        saver.to_thread = None;
        saver.thread.take().unwrap().join().unwrap();
        assert!(!path.exists());
        assert!(!saver.tick(&history, start + 2 * SAVE_DELAY));

        std::fs::create_dir(path.parent().unwrap()).unwrap();
        saver.finish(&history);
        let loaded = History::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.len(), history.len());
        for expected in &history.entries {
            let actual = loaded.get(&expected.url).unwrap();
            assert_eq!(
                (actual.url.as_str(), actual.visits, actual.last),
                (expected.url.as_str(), expected.visits, expected.last)
            );
        }
    }

    #[test]
    fn history_completion_cycles_through_matches() {
        let mut history = History::default();
        history.visit("https://third.example/", NOW);
        for _ in 0..2 {
            history.visit("https://second.example/", NOW);
        }
        for _ in 0..3 {
            history.visit("https://first.example/", NOW);
        }
        let mut completion = Completion::new(&history, "example", NOW);
        assert_eq!(completion.step(false), Some("https://first.example/"));
        assert_eq!(completion.step(false), Some("https://second.example/"));
        assert_eq!(completion.step(true), Some("https://first.example/"));
        assert_eq!(completion.step(true), Some("https://third.example/"));
        assert_eq!(Completion::new(&history, "nothing", NOW).step(false), None);
    }

    /// Release-mode speed gate: 10 000 entries (5x the cap), a 1-character query.
    #[test]
    fn history_filter_of_10000_entries_is_under_1_ms_at_p90() {
        let mut history = History::default();
        for i in 0..10_000u64 {
            let url = format!("https://site{i}.example.org/articles/{}/index.html", i * 7919);
            let title = format!("Article number {i} about topic {} - Example News", i % 97);
            history.entries.push(Entry::new(url, title, 1 + (i % 13) as u32, NOW - (i % 120) * DAY));
        }
        assert_eq!(history.len(), 10_000);
        let mut samples: Vec<std::time::Duration> = (0..100)
            .map(|_| {
                let start = std::time::Instant::now();
                let found = history.top(std::hint::black_box("e"), NOW);
                std::hint::black_box(&found);
                let took = start.elapsed();
                assert_eq!(found.len(), 9);
                took
            })
            .collect();
        samples.sort();
        let (p50, p90) = (samples[49], samples[89]);
        println!("history filter 10000 entries: p50 {p50:?} p90 {p90:?}");
        assert!(p90 < std::time::Duration::from_millis(1), "p90 {p90:?}");
    }
}
