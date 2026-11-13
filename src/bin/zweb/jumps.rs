#[cfg(test)]
mod tests {
    use super::{Jump, Jumps, CAP};

    fn jump(tab: u64, url: &str) -> Jump {
        Jump { tab, url: url.into() }
    }

    fn walked(jumps: &mut Jumps, current: Jump) -> Vec<Jump> {
        std::iter::successors(jumps.back(current), |_| jumps.back(jump(0, "")))
            .collect()
    }

    #[test]
    fn jumps_back_then_forward_returns_the_same_entry() {
        let mut jumps = Jumps::default();
        for j in [jump(1, "a"), jump(1, "b"), jump(2, "c")] {
            jumps.record(j);
        }
        assert_eq!(jumps.back(jump(2, "c")), Some(jump(1, "b")));
        assert_eq!(jumps.back(jump(1, "b")), Some(jump(1, "a")));
        assert_eq!(jumps.back(jump(1, "a")), None);
        assert_eq!(jumps.forward(), Some(jump(1, "b")));
        assert_eq!(jumps.forward(), Some(jump(2, "c")));
        assert_eq!(jumps.forward(), None);
        assert_eq!(jumps.back(jump(2, "c")), Some(jump(1, "b")));
    }

    #[test]
    fn jumps_back_records_the_current_entry_so_forward_can_return() {
        let mut jumps = Jumps::default();
        jumps.record(jump(1, "a"));
        assert_eq!(jumps.back(jump(2, "x")), Some(jump(1, "a")));
        assert_eq!(jumps.forward(), Some(jump(2, "x")));
    }

    #[test]
    fn jumps_record_after_back_drops_forward_entries() {
        let mut jumps = Jumps::default();
        for j in [jump(1, "a"), jump(1, "b"), jump(1, "c")] {
            jumps.record(j);
        }
        assert_eq!(jumps.back(jump(1, "c")), Some(jump(1, "b")));
        jumps.record(jump(1, "d"));
        assert_eq!(jumps.forward(), None);
        assert_eq!(walked(&mut jumps, jump(1, "d")), [jump(1, "b"), jump(1, "a")]);
    }

    #[test]
    fn jumps_collapse_consecutive_duplicates() {
        let mut jumps = Jumps::default();
        for j in [jump(1, "a"), jump(1, "a"), jump(1, "b"), jump(1, "b"), jump(1, "a")] {
            jumps.record(j);
        }
        assert_eq!(walked(&mut jumps, jump(1, "a")), [jump(1, "b"), jump(1, "a")]);
    }

    #[test]
    fn jumps_remove_tab_drops_only_that_tabs_entries() {
        let mut jumps = Jumps::default();
        for j in [jump(1, "a"), jump(2, "x"), jump(1, "b"), jump(2, "y"), jump(3, "z")] {
            jumps.record(j);
        }
        assert_eq!(jumps.back(jump(3, "z")), Some(jump(2, "y")));
        jumps.remove_tab(2);
        assert_eq!(walked(&mut jumps, jump(3, "z")), [jump(1, "b"), jump(1, "a")]);
        assert_eq!(jumps.forward(), Some(jump(1, "b")));
        assert_eq!(jumps.forward(), Some(jump(3, "z")));
        assert_eq!(jumps.forward(), None);
    }

    #[test]
    fn jumps_remove_tab_collapses_neighbours_that_become_equal() {
        let mut jumps = Jumps::default();
        for j in [jump(1, "a"), jump(2, "x"), jump(1, "a"), jump(3, "z")] {
            jumps.record(j);
        }
        jumps.remove_tab(2);
        assert_eq!(walked(&mut jumps, jump(3, "z")), [jump(1, "a")]);
    }

    #[test]
    fn jumps_cap_keeps_the_newest_entries() {
        let mut jumps = Jumps::default();
        for n in 0..CAP + 50 {
            jumps.record(jump(1, &n.to_string()));
        }
        let last = jump(1, &(CAP + 49).to_string());
        let earlier = walked(&mut jumps, last);
        assert_eq!(CAP, 100);
        assert_eq!(earlier.len(), CAP - 1);
        assert_eq!(earlier.first(), Some(&jump(1, &(CAP + 48).to_string())));
        assert_eq!(earlier.last(), Some(&jump(1, "50")));
    }
}
