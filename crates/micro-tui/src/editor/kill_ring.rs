//! The kill ring: what the cut commands cut, and what yank puts back.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LastAction {
    #[default]
    Other,
    Kill,
    Yank,
    /// A run of word characters, coalesced into one undo unit.
    TypeWord,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KillRing {
    entries: Vec<String>,
}

impl KillRing {
    pub fn new() -> Self {
        KillRing::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The text a yank would insert.
    pub fn peek(&self) -> Option<&str> {
        self.entries.last().map(String::as_str)
    }

    /// Record killed text.
    pub fn push(&mut self, text: &str, backward: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        match (accumulate, self.entries.last_mut()) {
            (true, Some(entry)) => match backward {
                true => entry.insert_str(0, text),
                false => entry.push_str(text),
            },
            _ => self.entries.push(text.to_string()),
        }
    }

    /// Rotate for yank-pop: the most recent entry goes to the front, and what was before it becomes
    /// what the next yank inserts.
    pub fn rotate(&mut self) -> Option<&str> {
        if self.entries.len() < 2 {
            return None;
        }
        let last = self.entries.pop()?;
        self.entries.insert(0, last);
        self.entries.last().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_yank_takes_the_most_recent_kill() {
        let mut ring = KillRing::new();
        ring.push("first", true, false);
        ring.push("second", true, false);
        assert_eq!(ring.peek(), Some("second"));
    }

    #[test]
    fn consecutive_backward_kills_read_in_buffer_order() {
        let mut ring = KillRing::new();
        ring.push("world", true, false);
        ring.push("hello ", true, true);
        assert_eq!(ring.peek(), Some("hello world"));
    }

    #[test]
    fn consecutive_forward_kills_read_in_buffer_order() {
        let mut ring = KillRing::new();
        ring.push("hello ", false, false);
        ring.push("world", false, true);
        assert_eq!(ring.peek(), Some("hello world"));
    }

    #[test]
    fn a_kill_that_does_not_accumulate_starts_its_own_entry() {
        let mut ring = KillRing::new();
        ring.push("one", true, false);
        ring.push("two", true, false);
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.peek(), Some("two"));
    }

    #[test]
    fn rotating_walks_back_through_the_ring() {
        let mut ring = KillRing::new();
        ring.push("one", true, false);
        ring.push("two", true, false);
        ring.push("three", true, false);

        assert_eq!(ring.rotate(), Some("two"));
        assert_eq!(ring.rotate(), Some("one"));
        assert_eq!(ring.rotate(), Some("three"));
    }

    #[test]
    fn a_ring_of_one_has_nothing_to_rotate_to() {
        let mut ring = KillRing::new();
        ring.push("only", true, false);
        assert_eq!(ring.rotate(), None);
        assert_eq!(ring.peek(), Some("only"));
    }

    #[test]
    fn killing_nothing_records_nothing() {
        let mut ring = KillRing::new();
        ring.push("", true, false);
        assert!(ring.is_empty());
    }
}
