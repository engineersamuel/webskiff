//! The jumplist: big moves across pages and tabs, walked with Ctrl-O / Ctrl-I.

pub const CAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
    pub tab: u64,
    pub url: String,
}

/// `at == list.len()` while not walking; otherwise `list[at]` is where we are.
#[derive(Debug, Default)]
pub struct Jumps {
    list: Vec<Jump>,
    at: usize,
}

impl Jumps {
    /// A new place: forget the forward entries, then add it unless it repeats the last one.
    pub fn record(&mut self, jump: Jump) {
        self.list.truncate((self.at + 1).min(self.list.len()));
        if self.list.last() != Some(&jump) {
            self.list.push(jump);
        }
        if self.list.len() > CAP {
            self.list.remove(0);
        }
        self.at = self.list.len();
    }

    /// The entry before this one. `current` is recorded first, so forward comes back to it.
    pub fn back(&mut self, current: Jump) -> Option<Jump> {
        if self.at == self.list.len() {
            self.record(current);
            self.at = self.list.len() - 1;
        }
        self.at = self.at.checked_sub(1)?;
        Some(self.list[self.at].clone())
    }

    pub fn forward(&mut self) -> Option<Jump> {
        if self.at + 1 >= self.list.len() {
            return None;
        }
        self.at += 1;
        Some(self.list[self.at].clone())
    }

    /// Drop a closed tab's entries; `at` stays on the same surviving entry.
    pub fn remove_tab(&mut self, tab: u64) {
        let walking = self.at < self.list.len();
        let mut at = 0;
        let mut kept: Vec<Jump> = Vec::with_capacity(self.list.len());
        for (i, jump) in std::mem::take(&mut self.list).into_iter().enumerate() {
            if i == self.at {
                at = kept.len();
            }
            if jump.tab != tab && kept.last() != Some(&jump) {
                kept.push(jump);
            }
        }
        self.list = kept;
        self.at = if walking {
            at.min(self.list.len().saturating_sub(1))
        } else {
            self.list.len()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{Jump, Jumps, CAP};

    fn jump(tab: u64, url: &str) -> Jump {
        Jump {
            tab,
            url: url.into(),
        }
    }

    fn walked(jumps: &mut Jumps, current: Jump) -> Vec<Jump> {
        std::iter::successors(jumps.back(current), |_| jumps.back(jump(0, ""))).collect()
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
        assert_eq!(
            walked(&mut jumps, jump(1, "d")),
            [jump(1, "b"), jump(1, "a")]
        );
    }

    #[test]
    fn jumps_collapse_consecutive_duplicates() {
        let mut jumps = Jumps::default();
        for j in [
            jump(1, "a"),
            jump(1, "a"),
            jump(1, "b"),
            jump(1, "b"),
            jump(1, "a"),
        ] {
            jumps.record(j);
        }
        assert_eq!(
            walked(&mut jumps, jump(1, "a")),
            [jump(1, "b"), jump(1, "a")]
        );
    }

    #[test]
    fn jumps_remove_tab_drops_only_that_tabs_entries() {
        let mut jumps = Jumps::default();
        for j in [
            jump(1, "a"),
            jump(2, "x"),
            jump(1, "b"),
            jump(2, "y"),
            jump(3, "z"),
        ] {
            jumps.record(j);
        }
        assert_eq!(jumps.back(jump(3, "z")), Some(jump(2, "y")));
        jumps.remove_tab(2);
        assert_eq!(
            walked(&mut jumps, jump(3, "z")),
            [jump(1, "b"), jump(1, "a")]
        );
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
