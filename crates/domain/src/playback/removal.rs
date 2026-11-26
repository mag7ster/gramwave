use super::{QueueEntry, QueueEntryId, Session};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Preceding,
    History,
    Remaining,
    Order,
}

struct Fragment {
    section: Section,
    start: usize,
    ids: Vec<QueueEntryId>,
    left: Vec<QueueEntryId>,
    right: Vec<QueueEntryId>,
}

impl Fragment {
    fn anchors(&self, right: bool) -> &[QueueEntryId] {
        if right {
            &self.right
        } else {
            &self.left
        }
    }

    fn restore(self, list: &mut Vec<QueueEntryId>, pending: &[&QueueRemoval]) {
        let locate = |right| {
            let mut candidates: Vec<_> = self.anchors(right).iter().rev().copied().collect();
            let mut seen = HashSet::new();
            while let Some(anchor) = candidates.pop() {
                if !seen.insert(anchor) {
                    continue;
                }
                if let Some(index) = list
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| **id == anchor)
                    .min_by_key(|(index, _)| index.abs_diff(self.start))
                    .map(|(index, _)| index)
                {
                    return Some(index);
                }
                if let Some(missing) =
                    pending
                        .iter()
                        .flat_map(|removal| &removal.fragments)
                        .find(|fragment| {
                            fragment.section == self.section && fragment.ids.contains(&anchor)
                        })
                {
                    candidates.extend(missing.anchors(right).iter().rev().copied());
                }
            }
            None
        };
        let at = locate(true)
            .or_else(|| locate(false).map(|index| index + 1))
            .unwrap_or_else(|| self.start.min(list.len()));
        list.splice(at..at, self.ids);
    }
}

pub struct QueueRemoval {
    fragments: Vec<Fragment>,
    entries: Vec<QueueEntry>,
    current: Option<(QueueEntryId, u64)>,
}

impl QueueRemoval {
    pub fn removed_current(&self) -> bool {
        self.current.is_some()
    }
}

fn remove_fragments(
    list: &mut Vec<QueueEntryId>,
    section: Section,
    ranges: impl DoubleEndedIterator<Item = std::ops::Range<usize>>,
    fragments: &mut Vec<Fragment>,
) {
    for range in ranges.rev() {
        fragments.push(Fragment {
            section,
            start: range.start,
            left: range
                .start
                .checked_sub(1)
                .map(|i| list[i])
                .into_iter()
                .collect(),
            right: list.get(range.end).copied().into_iter().collect(),
            ids: list.drain(range).collect(),
        });
    }
}

impl Session {
    pub fn remove_timeline_rows<'a>(
        &mut self,
        start: usize,
        expected: &[QueueEntryId],
        pending: impl DoubleEndedIterator<Item = &'a QueueRemoval>,
    ) -> Option<QueueRemoval> {
        let queue = self.timeline();
        let end = start.checked_add(expected.len())?;
        if expected.is_empty()
            || end > queue.len()
            || (start..end)
                .zip(expected)
                .any(|(index, id)| queue.get(index).unwrap().entry_id != *id)
        {
            return None;
        }

        let selected: HashSet<_> = expected.iter().copied().collect();
        let entries = selected.iter().map(|id| self.entries[id].clone()).collect();
        let current = queue
            .current_index()
            .filter(|i| (start..end).contains(i))
            .map(|_| (self.current.unwrap(), self.position_ms));
        let mut fragments = Vec::new();
        let upcoming = queue.upcoming_start();
        let preceding = self.preceding.len();
        for (section, base, list) in [
            (Section::Preceding, 0, &mut self.preceding),
            (Section::History, preceding, &mut self.history),
            (Section::Remaining, upcoming, &mut self.remaining),
        ] {
            let from = start.max(base).min(base + list.len()) - base;
            let to = end.max(base).min(base + list.len()) - base;
            if from < to {
                remove_fragments(list, section, std::iter::once(from..to), &mut fragments);
            }
        }
        if current.is_some() {
            self.current = None;
            self.position_ms = 0;
        }

        let still_visible: HashSet<_> = self
            .preceding
            .iter()
            .chain(&self.history)
            .chain(&self.current)
            .chain(&self.remaining)
            .copied()
            .collect();
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for (i, id) in self.order.iter().enumerate() {
            if selected.contains(id) && !still_visible.contains(id) {
                match ranges.last_mut() {
                    Some(range) if range.end == i => range.end += 1,
                    _ => ranges.push(i..i + 1),
                }
            }
        }
        remove_fragments(
            &mut self.order,
            Section::Order,
            ranges.into_iter(),
            &mut fragments,
        );
        self.collect();

        // A later removal can border a slot already hidden by an earlier one.
        // Keep that slot as the closer anchor if the earlier one is restored first.
        for previous in pending.rev() {
            for older in &previous.fragments {
                for new in fragments
                    .iter_mut()
                    .filter(|new| new.section == older.section)
                {
                    if older.right.first() == new.ids.first() {
                        let mut left: Vec<_> =
                            older.ids.iter().rev().chain(&older.left).copied().collect();
                        left.append(&mut new.left);
                        new.left = left;
                    }
                    if older.left.first() == new.ids.last() {
                        let mut right: Vec<_> =
                            older.ids.iter().chain(&older.right).copied().collect();
                        right.append(&mut new.right);
                        new.right = right;
                    }
                }
            }
        }
        Some(QueueRemoval {
            fragments,
            entries,
            current,
        })
    }

    pub fn restore_timeline_rows<'a>(
        &mut self,
        removal: QueueRemoval,
        pending: impl IntoIterator<Item = &'a QueueRemoval>,
    ) -> bool {
        let pending: Vec<_> = pending.into_iter().collect();
        for entry in removal.entries {
            self.entries.entry(entry.entry_id).or_insert(entry);
        }
        if let Some((id, position_ms)) = removal.current {
            if let Some(displaced) = self.current.replace(id) {
                self.remaining.insert(0, displaced);
            }
            self.position_ms = position_ms;
        }
        for mut fragment in removal.fragments.into_iter().rev() {
            let list = match fragment.section {
                Section::Preceding => &mut self.preceding,
                Section::History => &mut self.history,
                Section::Remaining => &mut self.remaining,
                Section::Order => {
                    fragment.ids.retain(|id| !self.order.contains(id));
                    &mut self.order
                }
            };
            fragment.restore(list, &pending);
        }
        removal.current.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackMetadata;

    fn track(message: i64) -> TrackMetadata {
        TrackMetadata {
            message,
            ..Default::default()
        }
    }

    fn orders(session: &Session) -> (Vec<QueueEntryId>, Vec<QueueEntryId>) {
        (
            session
                .remaining()
                .iter()
                .map(|entry| entry.entry_id)
                .collect(),
            session
                .snapshot()
                .iter()
                .map(|entry| entry.entry_id)
                .collect(),
        )
    }

    #[test]
    fn independent_adjacent_removals_restore_order_in_any_sequence() {
        let permutations = [
            [0, 1, 2, 3],
            [0, 1, 3, 2],
            [0, 2, 1, 3],
            [0, 2, 3, 1],
            [0, 3, 1, 2],
            [0, 3, 2, 1],
            [1, 0, 2, 3],
            [1, 0, 3, 2],
            [1, 2, 0, 3],
            [1, 2, 3, 0],
            [1, 3, 0, 2],
            [1, 3, 2, 0],
            [2, 0, 1, 3],
            [2, 0, 3, 1],
            [2, 1, 0, 3],
            [2, 1, 3, 0],
            [2, 3, 0, 1],
            [2, 3, 1, 0],
            [3, 0, 1, 2],
            [3, 0, 2, 1],
            [3, 1, 0, 2],
            [3, 1, 2, 0],
            [3, 2, 0, 1],
            [3, 2, 1, 0],
        ];
        for removal_order in permutations {
            for restore_order in permutations {
                let mut session = Session::default();
                session.insert_many((1..=4).map(track), false);
                let original = orders(&session);
                let mut pending = Vec::new();
                for i in removal_order {
                    let id = original.0[i];
                    let at = session
                        .remaining()
                        .iter()
                        .position(|entry| entry.entry_id == id)
                        .unwrap();
                    pending.push((
                        i,
                        session
                            .remove_timeline_rows(at, &[id], pending.iter().map(|(_, rows)| rows))
                            .unwrap(),
                    ));
                }
                for i in restore_order {
                    let at = pending.iter().position(|(track, _)| *track == i).unwrap();
                    let removal = pending.remove(at).1;
                    session.restore_timeline_rows(removal, pending.iter().map(|(_, rows)| rows));
                }
                assert_eq!(
                    orders(&session),
                    original,
                    "remove {removal_order:?}, restore {restore_order:?}"
                );
            }
        }
    }

    #[test]
    fn removal_across_current_restores_playback_checkpoint_and_keeps_later_edits() {
        let mut session = Session::default();
        session.start(&(1..=5).map(track).collect::<Vec<_>>(), 1, false);
        session.set_position_ms(37_000);
        let selected: Vec<_> = (0..3)
            .map(|index| session.timeline().get(index).unwrap().entry_id)
            .collect();
        assert!(session
            .remove_timeline_rows(1, &selected, std::iter::empty::<&QueueRemoval>())
            .is_none());
        let removal = session
            .remove_timeline_rows(0, &selected, std::iter::empty::<&QueueRemoval>())
            .unwrap();
        assert!(removal.removed_current());
        assert_eq!(session.jump_to(0).unwrap().message, 4);
        session.insert(track(6), false);
        assert!(session.restore_timeline_rows(removal, []));
        assert_eq!(session.current().unwrap().message, 2);
        assert_eq!(session.position_ms(), 37_000);
        assert_eq!(session.preceding().first().unwrap().message, 1);
        assert_eq!(
            session
                .remaining()
                .iter()
                .map(|entry| entry.message)
                .collect::<Vec<_>>(),
            [3, 4, 5, 6]
        );
    }

    #[test]
    fn removing_one_repeat_appearance_leaves_other_appearances_and_original_order() {
        let mut session = Session::default();
        session.start(&[track(1), track(2)], 0, false);
        session.advance(crate::RepeatMode::All, false, false);
        session.advance(crate::RepeatMode::All, false, false);
        let id = session.history().first().unwrap().entry_id;
        assert_eq!(session.current().unwrap().entry_id, id);
        let original = orders(&session);
        let removal = session
            .remove_timeline_rows(0, &[id], std::iter::empty::<&QueueRemoval>())
            .unwrap();
        assert_eq!(session.current().unwrap().entry_id, id);
        assert_eq!(orders(&session), original);
        session.restore_timeline_rows(removal, []);
        assert_eq!(session.history().first().unwrap().entry_id, id);
    }
}
