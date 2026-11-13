//! Visited pages, ranked by frecency, for completion in the Open prompts.

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
    fn history_saver_writes_the_last_snapshot_when_finished() {
        let dir = temp_dir("saver");
        let path = dir.join("history.tsv");
        let mut saver = Saver::spawn(path.clone(), std::time::Duration::from_secs(3600));
        let mut history = History::default();
        history.visit("https://first.example/", NOW);
        saver.save(&history);
        history.visit("https://second.example/", NOW);
        saver.save(&history);
        saver.finish();
        let loaded = History::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn history_completion_cycles_through_matches() {
        let mut history = History::default();
        history.visit("https://second.example/", NOW);
        for _ in 0..3 {
            history.visit("https://first.example/", NOW);
        }
        let mut completion = Completion::new(&history, "example", NOW);
        assert_eq!(completion.step(false), Some("https://first.example/"));
        assert_eq!(completion.step(false), Some("https://second.example/"));
        assert_eq!(completion.step(true), Some("https://first.example/"));
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
