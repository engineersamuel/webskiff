//! The jumplist: big moves across pages and tabs, walked with Ctrl-O / Ctrl-I.

pub const CAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jump {
    pub tab: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavOp {
    None,
    Load(String),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingJump {
    Restoring(String),
    Superseded(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabNav {
    pub id: u64,
    pub url: String,
    pending: Option<PendingJump>,
}

impl TabNav {
    pub fn new(id: u64, url: String) -> Self {
        Self {
            id,
            url,
            pending: None,
        }
    }

    pub fn current(&self) -> Jump {
        let url = match &self.pending {
            Some(PendingJump::Restoring(u)) => u.clone(),
            _ => self.url.clone(),
        };
        Jump { tab: self.id, url }
    }

    pub fn open(&mut self, url: String) {
        self.url = url;
        self.pending = None;
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }

    pub fn jump_to(&mut self, url: &str) -> NavOp {
        if self.url == url {
            if let Some(PendingJump::Restoring(old)) = self.pending.take() {
                self.pending = Some(PendingJump::Superseded(old));
                return NavOp::Stop;
            }
            NavOp::None
        } else {
            self.pending = Some(PendingJump::Restoring(url.into()));
            NavOp::Load(url.into())
        }
    }

    pub fn commit(&mut self, url: String, jumps: &mut Jumps) -> bool {
        match self.pending.take() {
            Some(PendingJump::Superseded(stale)) if stale == url => return false,
            Some(PendingJump::Restoring(want)) if want == url => {
                self.url = url;
            }
            _ => {
                self.url = url;
                if !self.url.is_empty() {
                    jumps.record(Jump {
                        tab: self.id,
                        url: self.url.clone(),
                    });
                }
            }
        }
        true
    }
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
            if jump.tab != tab && kept.last() != Some(&jump) {
                if i < self.at {
                    at += 1;
                }
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
        assert_eq!(jumps.back(jump(2, "y")), Some(jump(1, "b")));
        jumps.remove_tab(2);
        assert_eq!(jumps.back(jump(1, "b")), Some(jump(1, "a")));
        assert_eq!(jumps.back(jump(1, "a")), None);
        assert_eq!(jumps.forward(), Some(jump(1, "b")));
        assert_eq!(jumps.forward(), Some(jump(3, "z")));
        assert_eq!(jumps.forward(), None);

        let mut trailing = Jumps::default();
        for j in [jump(1, "a"), jump(2, "x")] {
            trailing.record(j);
        }
        assert_eq!(trailing.back(jump(2, "x")), Some(jump(1, "a")));
        assert_eq!(trailing.forward(), Some(jump(2, "x")));
        trailing.remove_tab(2);
        assert_eq!(trailing.forward(), None);
        assert_eq!(trailing.back(jump(1, "a")), None);
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

    #[test]
    fn superseding_in_flight_jump_back_to_committed_url_stops_pending_load_and_ignores_stale_commit(
    ) {
        let mut jumps = Jumps::default();
        jumps.record(jump(1, "a"));
        jumps.record(jump(1, "b"));
        let mut tab = super::TabNav::new(1, "b".into());
        let target_a = jumps.back(tab.current()).unwrap();
        assert_eq!(tab.jump_to(&target_a.url), super::NavOp::Load("a".into()));
        let target_b = jumps.forward().unwrap();
        assert_eq!(tab.jump_to(&target_b.url), super::NavOp::Stop);
        assert!(!tab.commit("a".into(), &mut jumps));
        assert_eq!(tab.url, "b");
        assert_eq!(jumps.back(tab.current()), Some(jump(1, "a")));
    }

    #[test]
    fn jump_commit_is_not_recorded_as_a_new_jump_but_user_navigation_is() {
        let mut jumps = Jumps::default();
        jumps.record(jump(1, "a"));
        jumps.record(jump(1, "b"));
        let mut tab = super::TabNav::new(1, "b".into());
        let target_a = jumps.back(tab.current()).unwrap();
        assert_eq!(tab.jump_to(&target_a.url), super::NavOp::Load("a".into()));
        assert!(tab.commit("a".into(), &mut jumps));
        assert_eq!(tab.jump_to(&target_a.url), super::NavOp::None);
        let target_b = jumps.forward().unwrap();
        assert_eq!(tab.jump_to(&target_b.url), super::NavOp::Load("b".into()));
        assert!(tab.commit("b".into(), &mut jumps));

        // User-initiated navigation while restore was in flight records the new jump.
        let target_a = jumps.back(tab.current()).unwrap();
        assert_eq!(tab.jump_to(&target_a.url), super::NavOp::Load("a".into()));
        assert_eq!(tab.current(), jump(1, "a"));
        tab.clear_pending();
        assert_eq!(tab.current(), jump(1, "b"));
        tab.open("c".into());
        assert_eq!(tab.url, "c");
        assert!(tab.commit("c".into(), &mut jumps));
        assert_eq!(jumps.forward(), None);
        assert_eq!(jumps.back(tab.current()), Some(jump(1, "a")));
    }

    #[test]
    fn cancelled_jump_does_not_suppress_later_navigation() {
        let mut jumps = Jumps::default();
        jumps.record(jump(1, "a"));
        jumps.record(jump(1, "b"));
        let mut tab = super::TabNav::new(1, "b".into());
        let target = jumps.back(tab.current()).unwrap();
        assert_eq!(tab.jump_to(&target.url), super::NavOp::Load("a".into()));

        // Failed loads and Stop both cancel the per-tab restoration.
        tab.clear_pending();
        assert_eq!(tab.current(), jump(1, "b"));
        assert!(tab.commit("a".into(), &mut jumps));
        assert_eq!(jumps.forward(), None);
        assert_eq!(jumps.back(tab.current()), None);
    }

    #[test]
    fn background_commit_preserves_forward_jump() {
        let mut jumps = Jumps::default();
        jumps.record(jump(1, "a"));
        let mut background = super::TabNav::new(2, "b".into());
        jumps.record(background.current());
        assert_eq!(jumps.back(background.current()), Some(jump(1, "a")));

        // Tab 1 is active when the slow second tab finally commits.
        assert!(background.commit("redirected-b".into(), &mut jumps));
        assert_eq!(background.current(), jump(2, "redirected-b"));
        assert_eq!(jumps.forward(), Some(jump(2, "b")));
        assert_eq!(jumps.back(jump(2, "b")), Some(jump(1, "a")));
    }
}
