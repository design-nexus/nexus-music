//! The play queue, kept in play order. Shuffling reorders it (current song
//! first) and remembers the original order so turning shuffle off restores it.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    pub fn id(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        }
    }

    pub fn from_id(id: &str) -> Repeat {
        match id {
            "all" => Repeat::All,
            "one" => Repeat::One,
            _ => Repeat::Off,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Queue {
    pub items: Vec<PathBuf>,
    pub cursor: Option<usize>,
    pub repeat: Repeat,
    /// The order before shuffling, while shuffle is on.
    original: Option<Vec<PathBuf>>,
    seed: u64,
}

impl Default for Queue {
    fn default() -> Self {
        Queue { items: Vec::new(), cursor: None, repeat: Repeat::Off, original: None, seed: seed() }
    }
}

fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    t | 1
}

impl Queue {
    pub fn shuffled(&self) -> bool {
        self.original.is_some()
    }

    pub fn current(&self) -> Option<&PathBuf> {
        self.cursor.and_then(|c| self.items.get(c))
    }

    /// xorshift64*: plenty for shuffling, no extra dependency.
    fn rand(&mut self, below: usize) -> usize {
        let mut x = self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) % below as u64) as usize
    }

    fn shuffle_slice(&mut self, v: &mut [PathBuf]) {
        for i in (1..v.len()).rev() {
            let j = self.rand(i + 1);
            v.swap(i, j);
        }
    }

    /// Replace the queue and start at `start` (shuffled after it when shuffle is on).
    pub fn set(&mut self, items: Vec<PathBuf>, start: usize) {
        let start = start.min(items.len().saturating_sub(1));
        if self.shuffled() {
            self.original = Some(items.clone());
            let mut rest = items;
            let first = if rest.is_empty() { None } else { Some(rest.remove(start)) };
            self.shuffle_slice(&mut rest);
            self.items = first.into_iter().chain(rest).collect();
            self.cursor = if self.items.is_empty() { None } else { Some(0) };
        } else {
            self.cursor = if items.is_empty() { None } else { Some(start) };
            self.items = items;
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffled() {
            return;
        }
        if on {
            self.original = Some(self.items.clone());
            let current = self.cursor.map(|c| self.items.remove(c));
            let mut rest = std::mem::take(&mut self.items);
            self.shuffle_slice(&mut rest);
            self.cursor = current.as_ref().map(|_| 0);
            self.items = current.into_iter().chain(rest).collect();
        } else if let Some(orig) = self.original.take() {
            let current = self.current().cloned();
            self.items = orig;
            self.cursor = current.and_then(|c| self.items.iter().position(|p| *p == c)).or(if self.items.is_empty() {
                None
            } else {
                Some(0)
            });
        }
    }

    /// Where playback goes after the current song. `manual` (the Next button)
    /// skips even when repeating one song.
    fn next_cursor(&self, manual: bool) -> Option<usize> {
        let c = self.cursor?;
        if self.repeat == Repeat::One && !manual {
            return Some(c);
        }
        if c + 1 < self.items.len() {
            Some(c + 1)
        } else if self.repeat != Repeat::Off && !self.items.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    /// What plays next on its own (for gapless pre-loading).
    pub fn peek_next(&self) -> Option<&PathBuf> {
        self.next_cursor(false).and_then(|c| self.items.get(c))
    }

    pub fn advance(&mut self, manual: bool) -> Option<PathBuf> {
        let next = self.next_cursor(manual)?;
        self.cursor = Some(next);
        self.current().cloned()
    }

    pub fn previous(&mut self) -> Option<PathBuf> {
        let c = self.cursor?;
        let prev = if c > 0 {
            c - 1
        } else if self.repeat != Repeat::Off {
            self.items.len().checked_sub(1)?
        } else {
            0
        };
        self.cursor = Some(prev);
        self.current().cloned()
    }

    pub fn jump(&mut self, index: usize) -> Option<PathBuf> {
        if index < self.items.len() {
            self.cursor = Some(index);
        }
        self.current().cloned()
    }

    pub fn append(&mut self, paths: &[PathBuf]) {
        if let Some(o) = &mut self.original {
            o.extend(paths.iter().cloned());
        }
        self.items.extend(paths.iter().cloned());
        if self.cursor.is_none() && !self.items.is_empty() {
            self.cursor = Some(self.items.len() - paths.len());
        }
    }

    /// Insert right after the current song.
    pub fn insert_next(&mut self, paths: &[PathBuf]) {
        let at = self.cursor.map_or(self.items.len(), |c| c + 1);
        if let Some(o) = &mut self.original {
            let cur = self.cursor.and_then(|c| self.items.get(c));
            let pos = cur.and_then(|c| o.iter().position(|p| p == c)).map_or(o.len(), |i| i + 1);
            o.splice(pos..pos, paths.iter().cloned());
        }
        self.items.splice(at..at, paths.iter().cloned());
        if self.cursor.is_none() && !self.items.is_empty() {
            self.cursor = Some(at);
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }
        let removed = self.items.remove(index);
        if let Some(o) = &mut self.original
            && let Some(i) = o.iter().position(|p| *p == removed)
        {
            o.remove(i);
        }
        self.cursor = match self.cursor {
            _ if self.items.is_empty() => None,
            Some(c) if index < c => Some(c - 1),
            Some(c) if c >= self.items.len() => Some(self.items.len() - 1),
            other => other,
        };
    }

    /// Move the items at `indexes` together to just before `to` (0..=len),
    /// keeping their order and the cursor on the same song.
    pub fn move_items(&mut self, indexes: &[usize], to: usize) {
        let order = block_move(self.items.len(), indexes, to);
        self.items = order.iter().map(|&i| self.items[i].clone()).collect();
        self.cursor = self.cursor.and_then(|c| order.iter().position(|&i| i == c));
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.cursor = None;
        if self.original.is_some() {
            self.original = Some(Vec::new());
        }
    }

    /// Restore a saved queue as-is.
    pub fn restore(&mut self, items: Vec<PathBuf>, cursor: Option<usize>, original: Option<Vec<PathBuf>>) {
        self.cursor = cursor.filter(|c| *c < items.len());
        self.items = items;
        self.original = original;
    }

    pub fn original(&self) -> Option<&Vec<PathBuf>> {
        self.original.as_ref()
    }
}

/// The new order (as old indexes) after moving `indexes` as one block to
/// just before position `to` of the old list.
pub fn block_move(len: usize, indexes: &[usize], to: usize) -> Vec<usize> {
    let mut moving: Vec<usize> = indexes.iter().copied().filter(|&i| i < len).collect();
    moving.sort_unstable();
    moving.dedup();
    let rest: Vec<usize> = (0..len).filter(|i| !moving.contains(i)).collect();
    let at = rest.iter().take_while(|&&i| i < to.min(len)).count();
    let mut out = rest[..at].to_vec();
    out.extend(&moving);
    out.extend(&rest[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_moves() {
        assert_eq!(block_move(5, &[3, 1], 0), vec![1, 3, 0, 2, 4]);
        assert_eq!(block_move(5, &[0, 1], 5), vec![2, 3, 4, 0, 1]);
        assert_eq!(block_move(5, &[0], 3), vec![1, 2, 0, 3, 4]);
        assert_eq!(block_move(5, &[2], 2), vec![0, 1, 2, 3, 4]);
        assert_eq!(block_move(3, &[], 1), vec![0, 1, 2]);
    }

    #[test]
    fn move_items_keeps_the_cursor() {
        let mut q = Queue::default();
        q.set(paths(5), 2);
        q.move_items(&[3, 4], 0);
        assert_eq!(q.items, vec!["/3", "/4", "/0", "/1", "/2"].into_iter().map(PathBuf::from).collect::<Vec<_>>());
        assert_eq!(q.current(), Some(&PathBuf::from("/2")));
    }

    fn paths(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("/{i}"))).collect()
    }

    #[test]
    fn plays_through_and_stops() {
        let mut q = Queue::default();
        q.set(paths(3), 1);
        assert_eq!(q.current(), Some(&PathBuf::from("/1")));
        assert_eq!(q.advance(false), Some(PathBuf::from("/2")));
        assert_eq!(q.advance(false), None);
        assert_eq!(q.cursor, Some(2));
    }

    #[test]
    fn repeats() {
        let mut q = Queue::default();
        q.set(paths(2), 1);
        q.repeat = Repeat::All;
        assert_eq!(q.peek_next(), Some(&PathBuf::from("/0")));
        q.repeat = Repeat::One;
        assert_eq!(q.peek_next(), Some(&PathBuf::from("/1")));
        // The Next button still moves on.
        assert_eq!(q.advance(true), Some(PathBuf::from("/0")));
    }

    #[test]
    fn shuffle_keeps_current_first_and_restores() {
        let mut q = Queue::default();
        q.set(paths(20), 5);
        q.set_shuffle(true);
        assert_eq!(q.cursor, Some(0));
        assert_eq!(q.current(), Some(&PathBuf::from("/5")));
        let mut sorted = q.items.clone();
        sorted.sort();
        let mut want = paths(20);
        want.sort();
        assert_eq!(sorted, want);
        q.advance(false);
        let now = q.current().cloned().unwrap();
        q.set_shuffle(false);
        assert_eq!(q.items, paths(20));
        assert_eq!(q.current(), Some(&now));
    }

    #[test]
    fn shuffled_set_starts_with_the_chosen_song() {
        let mut q = Queue::default();
        q.set_shuffle(true);
        q.set(paths(10), 7);
        assert_eq!(q.current(), Some(&PathBuf::from("/7")));
        assert_eq!(q.items.len(), 10);
    }

    #[test]
    fn inserts_next_and_removes() {
        let mut q = Queue::default();
        q.set(paths(3), 0);
        q.insert_next(&[PathBuf::from("/x")]);
        assert_eq!(q.items[1], PathBuf::from("/x"));
        q.remove(0);
        assert_eq!(q.cursor, Some(0));
        assert_eq!(q.current(), Some(&PathBuf::from("/x")));
        q.remove(0);
        assert_eq!(q.current(), Some(&PathBuf::from("/1")));
    }

    #[test]
    fn moves_keep_the_cursor_on_the_same_song() {
        let mut q = Queue::default();
        q.set(paths(4), 2);
        q.move_items(&[0], 4);
        assert_eq!(q.current(), Some(&PathBuf::from("/2")));
        q.move_items(&[q.cursor.unwrap()], 0);
        assert_eq!(q.cursor, Some(0));
        assert_eq!(q.current(), Some(&PathBuf::from("/2")));
    }

    #[test]
    fn previous_wraps_only_when_repeating() {
        let mut q = Queue::default();
        q.set(paths(3), 0);
        assert_eq!(q.previous(), Some(PathBuf::from("/0")));
        q.repeat = Repeat::All;
        assert_eq!(q.previous(), Some(PathBuf::from("/2")));
    }
}
