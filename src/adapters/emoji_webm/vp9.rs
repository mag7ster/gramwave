use super::Decoding;
use image::RgbaImage;
use rusty_vp9::{
    parse_uncompressed_header, BitReader, DecodedFrame, Error, FrameHeader, Vp9Decoder,
};

#[derive(Clone, Copy, Default)]
struct Colour {
    full: bool,
    hd: bool,
}

#[derive(Default)]
pub(super) struct Decoder {
    decoder: Vp9Decoder,
    references: [(u32, u32); 8],
    colour: Colour,
    reference_colours: [Colour; 8],
    coded_frames: usize,
}

pub(super) struct Picture(DecodedFrame, Colour);

impl Decoder {
    pub(super) fn packet(&mut self, packet: &[u8], state: &Decoding) -> Option<Option<Picture>> {
        let frames = coded_frames(packet)?;
        self.coded_frames += frames.len();
        if self.coded_frames > 360 {
            return None;
        }
        // Validate hidden frames and inter-frame sizes before the decoder allocates planes.
        let mut displayed = self.colour;
        for bytes in &frames {
            if !state.running() {
                return None;
            }
            let h = parse_uncompressed_header(&mut BitReader::new(bytes), &self.references).ok()?;
            if h.show_existing_frame {
                let slot = h.frame_to_show as usize;
                if self.references[slot] == (0, 0) {
                    return None;
                }
                displayed = self.reference_colours[slot];
                continue;
            }
            if !(1..=512).contains(&h.width) || !(1..=512).contains(&h.height) {
                return None;
            }
            if h.key_frame || h.intra_only {
                self.colour = Colour {
                    full: full_range(bytes, &h)?,
                    hd: h.color_space == 2,
                };
            }
            if h.show_frame {
                displayed = self.colour;
            }
            for (slot, size) in self.references.iter_mut().enumerate() {
                if h.key_frame || h.refresh_frame_flags & (1 << slot) != 0 {
                    *size = (h.width, h.height);
                    self.reference_colours[slot] = self.colour;
                }
            }
        }
        self.decoder.push(packet, None).ok()?;
        let mut picture = None;
        for _ in frames {
            if !state.running() {
                return None;
            }
            match self.decoder.next_frame() {
                Ok(frame) => picture = Some(Picture(frame, displayed)),
                Err(Error::Again) => (),
                Err(_) => return None,
            }
        }
        Some(picture)
    }
}

fn coded_frames(mut payload: &[u8]) -> Option<Vec<&[u8]>> {
    let marker = *payload.last()?;
    if marker & 0xe0 != 0xc0 {
        return Some(vec![payload]);
    }
    let count = usize::from(marker & 7) + 1;
    let width = usize::from((marker >> 3) & 3) + 1;
    let Some(start) = payload.len().checked_sub(2 + count * width) else {
        return Some(vec![payload]);
    };
    if payload[start] != marker {
        return Some(vec![payload]);
    }
    let index = &payload[start + 1..payload.len() - 1];
    payload = &payload[..start];
    let mut frames = Vec::with_capacity(count);
    for size in index.chunks_exact(width) {
        let mut bytes = [0; 4];
        bytes[..width].copy_from_slice(size);
        let size = u32::from_le_bytes(bytes) as usize;
        if size == 0 {
            return None;
        }
        let (frame, rest) = payload.split_at_checked(size)?;
        frames.push(frame);
        payload = rest;
    }
    payload.is_empty().then_some(frames)
}

fn full_range(bytes: &[u8], header: &FrameHeader) -> Option<bool> {
    if header.color_space == 7 {
        return Some(true);
    }
    if header.intra_only && header.profile == 0 {
        return Some(false);
    }
    // The public header omits color_range. Read this single bit from color_config.
    let mut offset = 8 + u32::from(header.profile == 3);
    if !header.key_frame {
        offset += u32::from(!header.show_frame) + 2 * u32::from(!header.error_resilient);
    }
    offset += 24 + u32::from(header.profile >= 2) + 3;
    Some(*bytes.get((offset / 8) as usize)? & (0x80 >> (offset % 8)) != 0)
}

fn sample(frame: &DecodedFrame, plane: usize, x: usize, y: usize) -> u8 {
    let bytes = if frame.bit_depth == 8 { 1 } else { 2 };
    let offset = y * frame.strides[plane] + bytes * x;
    let data = &frame.planes[plane][offset..offset + bytes];
    if bytes == 1 {
        data[0]
    } else {
        (u16::from_le_bytes([data[0], data[1]]) >> (frame.bit_depth - 8)) as u8
    }
}

impl Picture {
    pub(super) fn rgba(self, mask: Option<Self>) -> Option<RgbaImage> {
        let Self(frame, Colour { full, hd }) = self;
        let mask = mask.map(|mask| mask.0);
        if mask
            .as_ref()
            .is_some_and(|m| (m.width, m.height) != (frame.width, frame.height))
        {
            return None;
        }
        let (vr, ug, vg, ub) = if hd {
            (1.5748, 0.1873, 0.4681, 1.8556)
        } else {
            (1.402, 0.3441, 0.7141, 1.772)
        };
        let chroma_scale = if full { 1. } else { 255. / 224. };
        let mut pixels = RgbaImage::new(frame.width, frame.height);
        for (x, y, dest) in pixels.enumerate_pixels_mut() {
            let (x, y) = (x as usize, y as usize);
            let luma = f64::from(sample(&frame, 0, x, y));
            let luma = if full {
                luma
            } else {
                (luma - 16.) * 255. / 219.
            };
            let cx = x >> frame.subsampling_x;
            let cy = y >> frame.subsampling_y;
            let u = (f64::from(sample(&frame, 1, cx, cy)) - 128.) * chroma_scale;
            let v = (f64::from(sample(&frame, 2, cx, cy)) - 128.) * chroma_scale;
            *dest = image::Rgba([
                (luma + vr * v).round() as u8,
                (luma - ug * u - vg * v).round() as u8,
                (luma + ub * u).round() as u8,
                mask.as_ref().map_or(255, |m| sample(m, 0, x, y)),
            ]);
        }
        Some(pixels)
    }
}
