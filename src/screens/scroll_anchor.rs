/// A small set of rows near the middle of a viewport, with their old screen offsets.
#[derive(Clone)]
pub(super) struct ScrollAnchor<K> {
    rows: Vec<(K, f32)>,
    height: f32,
}

impl<K> ScrollAnchor<K> {
    pub(super) fn capture(
        len: usize,
        stride: f32,
        offset: f32,
        height: f32,
        key: impl Fn(usize) -> K,
    ) -> Self {
        let first = (offset / stride) as usize;
        let end = (((offset + height) / stride).ceil() as usize).min(len);
        let center = offset + height / 2.;
        let mut rows: Vec<_> = (first..end).collect();
        rows.sort_by_key(|&i| ((i as f32 + 0.5) * stride - center).abs().to_bits());
        Self {
            rows: rows
                .into_iter()
                .map(|i| (key(i), i as f32 * stride - offset))
                .collect(),
            height,
        }
    }

    pub(super) fn offset(
        &self,
        stride: f32,
        max: f32,
        find: impl Fn(&K) -> Option<usize>,
    ) -> Option<f32> {
        self.rows.iter().find_map(|(key, y)| {
            find(key).map(|index| (index as f32 * stride - y).clamp(0., max.max(0.)))
        })
    }

    pub(super) fn mirrored_offset(
        &self,
        stride: f32,
        max: f32,
        height: f32,
        row_height: f32,
        find: impl Fn(&K) -> Option<usize>,
    ) -> Option<f32> {
        let offset_for = |key: &K, new_top: f32| {
            find(key).map(|index| (index as f32 * stride - new_top).clamp(0., max.max(0.)))
        };
        self.rows
            .iter()
            .find_map(|(key, top)| {
                let bottom = top + row_height;
                // Transfer the clipped part of the row to the opposite edge.
                if *top < 0. && bottom > 0. {
                    offset_for(key, height - bottom)
                } else if *top < self.height && bottom > self.height {
                    offset_for(key, self.height - bottom)
                } else {
                    None
                }
            })
            .or_else(|| {
                self.rows
                    .iter()
                    .filter(|(_, top)| *top >= 0. && top + row_height <= self.height)
                    .find_map(|(key, top)| offset_for(key, height - (top + row_height)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollAnchor;

    #[test]
    fn reverse_mirrors_viewport_edges_in_row_spacing() {
        let stride = 68.;
        let height = 3. * stride - 2.;
        let anchor = ScrollAnchor::capture(30, stride, 8. * stride - 4., height, |i| i);
        let offset =
            anchor.mirrored_offset(stride, 30. * stride - height, height, 60., |i| Some(29 - i));
        assert_eq!(offset, Some(19. * stride - 2.));
    }
}
