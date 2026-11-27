use super::emoji::{Animation, FPS, MAX_FILE, MAX_FRAMES, SIZE};
use eframe::egui::ColorImage;
use image::{imageops, RgbaImage};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use webm_iterable::{
    matroska_spec::{Master, MatroskaSpec::*},
    WebmIterator,
};

mod vp9;

const TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn decode(raw: Vec<u8>, cancel: &CancellationToken) -> Option<Animation> {
    if raw.is_empty() || raw.len() > MAX_FILE {
        return None;
    }
    let child = cancel.child_token();
    let _cancel_on_return = child.clone().drop_guard();
    let mut worker = tokio::task::spawn_blocking(move || decode_sync(&raw, child));
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(TIMEOUT, &mut worker) => result.ok()?.ok()?,
    }
}

struct Decoding {
    stop: CancellationToken,
    deadline: Instant,
    frames: Vec<ColorImage>,
    pending: Option<(usize, f64, f64, ColorImage)>,
    previous_time: Option<f64>,
}

impl Decoding {
    fn running(&self) -> bool {
        !self.stop.is_cancelled() && Instant::now() < self.deadline
    }
}

fn frame(state: &mut Decoding, image: RgbaImage, timestamp: f64, duration: f64) -> Option<()> {
    let (width, height) = image.dimensions();
    if !state.running() || !timestamp.is_finite() || !duration.is_finite() {
        return None;
    }
    let slot = ((timestamp * f64::from(FPS)).round() as usize).min(MAX_FRAMES);
    if let Some((previous, time, _, image)) = state.pending.take() {
        if slot < previous || timestamp < time {
            return None;
        }
        state.previous_time = Some(time);
        state.frames.resize(slot, image);
    }
    let size = SIZE as u32;
    let longest = width.max(height);
    let scaled_width = (width * size).div_ceil(longest).max(1);
    let scaled_height = (height * size).div_ceil(longest).max(1);
    let scaled = imageops::resize(
        &image,
        scaled_width,
        scaled_height,
        imageops::FilterType::Triangle,
    );
    let mut canvas = RgbaImage::new(size, size);
    imageops::replace(
        &mut canvas,
        &scaled,
        i64::from((size - scaled_width) / 2),
        i64::from((size - scaled_height) / 2),
    );
    let image = ColorImage::from_rgba_unmultiplied([SIZE; 2], canvas.as_raw());
    state.pending = Some((slot, timestamp, duration, image));
    Some(())
}

fn decode_sync(raw: &[u8], stop: CancellationToken) -> Option<Animation> {
    let mut state = Decoding {
        stop,
        deadline: Instant::now() + TIMEOUT,
        frames: Vec::new(),
        pending: None,
        previous_time: None,
    };
    decode_blocks(raw, &mut state)?;
    if !state.running() {
        return None;
    }
    let (slot, time, duration, last) = state.pending.take()?;
    let duration = if duration > 0. {
        duration
    } else {
        state
            .previous_time
            .map_or(1. / f64::from(FPS), |previous| time - previous)
    };
    let end = (((time + duration) * f64::from(FPS)).round() as usize)
        .max(slot + 1)
        .min(MAX_FRAMES);
    state.frames.resize(end, last);
    Some(Animation {
        duration: Duration::from_secs_f64(state.frames.len() as f64 / f64::from(FPS)),
        frames: state.frames,
    })
}

fn decode_blocks(raw: &[u8], state: &mut Decoding) -> Option<()> {
    let mut tags = WebmIterator::new(raw, &[TrackEntry(Master::Start), BlockGroup(Master::Start)]);
    tags.set_max_allowable_tag_size(Some(raw.len()));
    let mut track = None;
    let mut default_duration = 0.;
    let mut alpha_required = false;
    let mut scale = 0.001;
    let mut cluster = 0;
    let mut colour = vp9::Decoder::default();
    let mut alpha = vp9::Decoder::default();
    let mut displayed = 0;
    for tag in tags {
        if !state.running() {
            return None;
        }
        let mut mask = None;
        let mut duration = default_duration;
        let block;
        let simple;
        let simple_data;
        let block_data;
        let (number, timestamp, laced, packet) = match tag.ok()? {
            TimestampScale(ns) if ns > 0 => {
                scale = ns as f64 / 1e9;
                continue;
            }
            TimestampScale(_) => return None,
            TrackEntry(Master::Full(fields)) if track.is_none() => {
                if !fields.contains(&TrackType(1)) {
                    continue;
                }
                if !fields.contains(&CodecID("V_VP9".into())) {
                    return None;
                }
                track = fields.iter().find_map(|field| match field {
                    TrackNumber(number) if *number > 0 => Some(*number),
                    _ => None,
                });
                track?;
                let mut video = None;
                for field in fields {
                    match field {
                        DefaultDuration(ns) => default_duration = ns as f64 / 1e9,
                        Video(Master::Full(fields)) => video = Some(fields),
                        _ => (),
                    }
                }
                let video = video?;
                if !video.iter().any(|f| matches!(f, PixelWidth(1..=512)))
                    || !video.iter().any(|f| matches!(f, PixelHeight(1..=512)))
                {
                    return None;
                }
                alpha_required = video.contains(&AlphaMode(1));
                continue;
            }
            Cluster(Master::Start) => {
                cluster = 0;
                continue;
            }
            Timestamp(ticks) => {
                cluster = ticks;
                continue;
            }
            SimpleBlock(data) => {
                simple_data = data;
                simple =
                    webm_iterable::matroska_spec::SimpleBlock::try_from(simple_data.as_slice())
                        .ok()?;
                (
                    simple.track,
                    simple.timestamp,
                    simple.lacing.is_some(),
                    simple.raw_frame_data(),
                )
            }
            BlockGroup(Master::Full(fields)) => {
                let mut data = None;
                for field in fields {
                    match field {
                        Block(bytes) => data = Some(bytes),
                        BlockDuration(ticks) => duration = ticks as f64 * scale,
                        BlockAdditions(Master::Full(additions)) => {
                            for addition in additions {
                                if let BlockMore(Master::Full(fields)) = addition {
                                    let id = fields
                                        .iter()
                                        .find_map(|field| match field {
                                            BlockAddID(id) => Some(*id),
                                            _ => None,
                                        })
                                        .unwrap_or(1);
                                    if id == 1 {
                                        let bytes =
                                            fields.into_iter().find_map(|field| match field {
                                                BlockAdditional(bytes) => Some(bytes),
                                                _ => None,
                                            })?;
                                        if mask.replace(bytes).is_some() {
                                            return None;
                                        }
                                    }
                                }
                            }
                        }
                        _ => (),
                    }
                }
                block_data = data?;
                block =
                    webm_iterable::matroska_spec::Block::try_from(block_data.as_slice()).ok()?;
                (
                    block.track,
                    block.timestamp,
                    block.lacing.is_some(),
                    block.raw_frame_data(),
                )
            }
            _ => continue,
        };
        if track != Some(number) {
            continue;
        }
        if laced || (alpha_required && mask.is_none()) {
            return None;
        }
        let picture = colour.packet(packet, state)?;
        let mask_expected = mask.is_some();
        let mask = match mask {
            Some(mask) => alpha.packet(&mask, state)?,
            None => None,
        };
        if let Some(picture) = picture {
            displayed += 1;
            if displayed > 90 || (mask_expected && mask.is_none()) {
                return None;
            }
            let ticks = cluster.checked_add_signed(i64::from(timestamp))?;
            frame(state, picture.rgba(mask)?, ticks as f64 * scale, duration)?;
        } else if mask.is_some() {
            return None;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE: &[u8] = include_bytes!("emoji_webm/edge.webm");

    #[tokio::test]
    async fn webm_frames_preserve_order_timing_and_partial_alpha() {
        let animation = decode(EDGE.to_vec(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(animation.duration, Duration::from_millis(500));
        assert_eq!(animation.frames.len(), 12);
        let center = 32 * SIZE;
        let first = &animation.frames[0].pixels;
        let last = &animation.frames[11].pixels;
        assert_eq!(first[0].a(), 0);
        assert_eq!(first[center + 14].a(), 255);
        assert_eq!(last[center + 14].a(), 0);
        assert_eq!(last[center + 48].a(), 255);
        assert!(first[center + 6..center + 11]
            .iter()
            .any(|pixel| (96..=160).contains(&pixel.a())));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(decode(EDGE.to_vec(), &cancel).await.is_none());
        assert!(decode(EDGE[..30].to_vec(), &CancellationToken::new())
            .await
            .is_none());
    }

    #[tokio::test]
    async fn vp9_dimensions_are_checked_even_when_the_container_claims_a_small_video() {
        let group = WebmIterator::new(EDGE, &[BlockGroup(Master::Start)])
            .find_map(|tag| match tag.unwrap() {
                BlockGroup(Master::Full(group)) => Some(group),
                _ => None,
            })
            .unwrap();
        let bytes = group
            .iter()
            .find_map(|tag| match tag {
                Block(bytes) => Some(bytes),
                _ => None,
            })
            .unwrap();
        let block = webm_iterable::matroska_spec::Block::try_from(bytes.as_slice()).unwrap();
        let packet = block.raw_frame_data();
        assert_eq!(&packet[..4], &[0x82, 0x49, 0x83, 0x42]);
        let start = EDGE
            .windows(packet.len())
            .position(|bytes| bytes == packet)
            .unwrap();
        let mut raw = EDGE.to_vec();
        // This profile 0 key frame stores width-minus-one in bits 36 through 51.
        raw[start + 4] |= 0x0f;
        raw[start + 5] = 0xff;
        raw[start + 6] |= 0xf0;
        assert!(decode(raw, &CancellationToken::new()).await.is_none());
    }
}
