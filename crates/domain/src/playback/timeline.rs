use super::{QueueEntry, QueueEntryId, QueueView, Session};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueRangeEdit {
    MoveUp,
    MoveDown,
    Reverse,
}

impl QueueRangeEdit {
    pub fn destination(self, start: usize, count: usize, len: usize) -> Option<usize> {
        let end = start.checked_add(count)?;
        if count == 0 || end > len {
            return None;
        }
        match self {
            Self::MoveUp => start.checked_sub(1),
            Self::MoveDown => (end < len).then_some(start + 1),
            Self::Reverse => (count > 1).then_some(start),
        }
    }
}

#[derive(Clone, Copy)]
pub struct QueueTimeline<'a> {
    preceding: QueueView<'a>,
    history: QueueView<'a>,
    current: Option<&'a QueueEntry>,
    remaining: QueueView<'a>,
}

impl<'a> QueueTimeline<'a> {
    pub fn past_len(self) -> usize {
        self.preceding.len() + self.history.len()
    }

    pub fn current_index(self) -> Option<usize> {
        self.current.map(|_| self.past_len())
    }

    pub fn upcoming_start(self) -> usize {
        self.past_len() + usize::from(self.current.is_some())
    }

    pub fn len(self) -> usize {
        self.upcoming_start() + self.remaining.len()
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn upcoming_ids(self) -> impl Iterator<Item = QueueEntryId> + 'a {
        self.remaining.ids.iter().copied()
    }

    pub fn get(self, index: usize) -> Option<&'a QueueEntry> {
        if index < self.preceding.len() {
            self.preceding.get(index)
        } else if index < self.past_len() {
            self.history.get(index - self.preceding.len())
        } else if index < self.upcoming_start() {
            self.current
        } else {
            self.remaining.get(index - self.upcoming_start())
        }
    }
}

impl Session {
    pub fn timeline(&self) -> QueueTimeline<'_> {
        QueueTimeline {
            preceding: self.preceding(),
            history: self.history(),
            current: self.current(),
            remaining: self.remaining(),
        }
    }

    pub fn play_timeline_row(
        &mut self,
        index: usize,
        expected: QueueEntryId,
    ) -> Option<QueueEntry> {
        let timeline = self.timeline();
        if timeline.get(index)?.entry_id != expected {
            return None;
        }
        if index < timeline.past_len() {
            self.rewind_to(index, expected)
        } else {
            self.jump_to(index.checked_sub(timeline.upcoming_start())?)
        }
    }

    fn rewind_to(&mut self, index: usize, expected: QueueEntryId) -> Option<QueueEntry> {
        let timeline = self.timeline();
        let past = timeline.past_len();
        if index + 1 == past {
            return self.previous();
        }

        let mut removed = HashSet::new();
        let mut remaining = Vec::new();
        // Rebuild the suffix with the same remove-then-prepend rule as Previous.
        for at in index..past {
            if at + 1 < timeline.upcoming_start() {
                let next = timeline.get(at + 1).unwrap();
                if !removed.contains(&next.entry_id) {
                    remaining.push(next.entry_id);
                }
            }
            removed.insert(timeline.get(at).unwrap().entry_id);
        }
        remaining.extend(self.remaining.iter().filter(|id| !removed.contains(id)));
        self.preceding.truncate(index);
        self.history.truncate(index - self.preceding.len());
        self.remaining = remaining;
        self.current = Some(expected);
        self.position_ms = 0;
        self.current().cloned()
    }

    pub fn move_timeline_row(&mut self, from: usize, expected: QueueEntryId, to: usize) {
        let timeline = self.timeline();
        if from == to
            || to >= timeline.len()
            || timeline
                .get(from)
                .is_none_or(|entry| entry.entry_id != expected)
        {
            return;
        }
        let upcoming = timeline.upcoming_start();
        if from >= upcoming && to >= upcoming {
            self.move_before(from - upcoming, to - upcoming + usize::from(to > from));
            return;
        }

        let old_past = timeline.past_len();
        let has_current = timeline.current_index().is_some();
        let mut rows = self.timeline_rows();
        let moved = rows.remove(from);
        rows.insert(to, moved);
        let past = if has_current {
            if from == old_past {
                to
            } else {
                old_past + usize::from(from > old_past && to <= old_past)
                    - usize::from(from < old_past && to >= old_past)
            }
        } else {
            old_past + usize::from(from >= old_past && to < old_past)
                - usize::from(from < old_past && to >= old_past)
        };
        self.apply_timeline_order(rows, past, from..from + 1);
    }

    pub fn edit_timeline_range(
        &mut self,
        start: usize,
        expected: &[QueueEntryId],
        edit: QueueRangeEdit,
    ) -> bool {
        let timeline = self.timeline();
        let Some(destination) = edit.destination(start, expected.len(), timeline.len()) else {
            return false;
        };
        let end = start + expected.len();
        if (start..end)
            .zip(expected)
            .any(|(i, &id)| timeline.get(i).unwrap().entry_id != id)
        {
            return false;
        }
        if expected.len() == 1 {
            self.move_timeline_row(start, expected[0], destination);
            return true;
        }
        let mut rows = self.timeline_rows();
        let past = timeline.past_len();
        match edit {
            QueueRangeEdit::MoveUp => rows[start - 1..end].rotate_left(1),
            QueueRangeEdit::MoveDown => rows[start..=end].rotate_right(1),
            QueueRangeEdit::Reverse => rows[start..end].reverse(),
        }
        let new_past = timeline.current_index().map_or(past, |current| {
            rows.iter()
                .position(|&(_, origin)| origin == current)
                .unwrap()
        });
        self.apply_timeline_order(rows, new_past, start..end);
        true
    }

    fn timeline_rows(&self) -> Vec<(QueueEntryId, usize)> {
        let timeline = self.timeline();
        (0..timeline.len())
            .map(|i| (timeline.get(i).unwrap().entry_id, i))
            .collect()
    }

    fn apply_timeline_order(
        &mut self,
        mut rows: Vec<(QueueEntryId, usize)>,
        past: usize,
        moved: std::ops::Range<usize>,
    ) {
        let old_past = self.timeline().past_len();
        let upcoming = past + usize::from(self.current.is_some());
        let mut seen = HashSet::new();
        let repeated: HashSet<_> = rows
            .iter()
            .filter_map(|&(id, _)| (!seen.insert(id)).then_some(id))
            .collect();
        let mut occupied: HashSet<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, &(id, origin))| (i < upcoming || origin >= old_past).then_some(id))
            .collect();
        let in_order: HashSet<_> = self.order.iter().copied().collect();
        let mut order_changes = HashMap::new();
        for (i, (id, origin)) in rows.iter_mut().enumerate() {
            let was_past = *origin < old_past;
            let moved = moved.contains(origin);
            if i >= upcoming && was_past {
                if !occupied.insert(*id) {
                    *id = self.entry(self.entries[id].track.clone());
                }
                if moved || !in_order.contains(id) {
                    order_changes.entry(*id).or_insert(i);
                }
            } else if moved && in_order.contains(id) && (!was_past || !repeated.contains(id)) {
                order_changes.entry(*id).or_insert(i);
            }
        }
        if !order_changes.is_empty() {
            // Repeat and unshuffle keep the relative order of entries the user didn't move.
            let mut before: HashMap<_, Vec<_>> = HashMap::new();
            let mut next = None;
            for (i, &(id, _)) in rows.iter().enumerate().rev() {
                if let Some(&at) = order_changes.get(&id) {
                    if i == at {
                        before.entry(next).or_default().push(id);
                    }
                } else if in_order.contains(&id) {
                    next = Some(id);
                }
            }
            let mut order = Vec::with_capacity(self.order.len());
            for &id in &self.order {
                if !order_changes.contains_key(&id) {
                    order.extend(before.remove(&Some(id)).into_iter().flatten().rev());
                    order.push(id);
                }
            }
            order.extend(before.remove(&None).into_iter().flatten().rev());
            self.order = order;
        }
        let preceding = self.preceding.len().min(past);
        self.preceding = rows[..preceding].iter().map(|&(id, _)| id).collect();
        self.history = rows[preceding..past].iter().map(|&(id, _)| id).collect();
        self.remaining = rows[upcoming..].iter().map(|&(id, _)| id).collect();
    }
}
