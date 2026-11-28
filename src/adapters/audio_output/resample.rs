use rubato::{
    audioadapter_buffers::direct::InterleavedSlice, Async, FixedAsync, Indexing, Resampler,
    SincInterpolationParameters,
};
use std::{
    num::{NonZeroU16, NonZeroU32},
    ops::Range,
};
use tokio_util::sync::CancellationToken;

pub(super) struct Resampled {
    resampler: Async<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
    ready: Range<usize>,
    from: u128,
    to: u128,
    frames_in: u128,
    frames_out: u128,
    delay: usize,
    eof: bool,
}

impl Resampled {
    pub(super) fn new(
        from: NonZeroU32,
        to: NonZeroU32,
        channels: NonZeroU16,
    ) -> Result<Self, String> {
        let channels = usize::from(channels.get());
        let resampler = Async::new_sinc(
            f64::from(to.get()) / f64::from(from.get()),
            1.,
            &SincInterpolationParameters::default(),
            512,
            channels,
            FixedAsync::Input,
        )
        .map_err(|error| error.to_string())?;
        // Media headers can declare rates that would require an oversized scratch buffer.
        let output_samples = resampler
            .output_frames_max()
            .checked_mul(channels)
            .filter(|&samples| samples <= 1_048_576)
            .ok_or("Audio resampling buffer is too large")?;
        Ok(Self {
            input: Vec::with_capacity(resampler.input_frames_max() * channels),
            output: vec![0.; output_samples],
            ready: 0..0,
            from: u128::from(from.get()),
            to: u128::from(to.get()),
            frames_in: 0,
            frames_out: 0,
            delay: resampler.output_delay(),
            eof: false,
            resampler,
        })
    }

    pub(super) fn next(
        &mut self,
        input: &mut impl Iterator<Item = f32>,
        cancel: &CancellationToken,
    ) -> Result<Option<f32>, String> {
        while self.ready.is_empty() {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let channels = self.resampler.nbr_channels();
            let needed = self.resampler.input_frames_next();
            self.input.clear();
            if !self.eof {
                self.input.extend(input.by_ref().take(needed * channels));
            }
            let frames = self.input.len() / channels;
            self.frames_in += frames as u128;
            self.eof = frames < needed;
            // Limit the output to real input frames, including while flushing with silence.
            let remaining = (self.frames_in * self.to)
                .div_ceil(self.from)
                .saturating_sub(self.frames_out);
            if self.eof && remaining == 0 {
                return Ok(None);
            }

            let indexing = Indexing {
                partial_len: self.eof.then_some(frames),
                ..Default::default()
            };
            let input = InterleavedSlice::new(&self.input, channels, frames).unwrap();
            let output_frames = self.output.len() / channels;
            let mut output =
                InterleavedSlice::new_mut(&mut self.output, channels, output_frames).unwrap();
            let (_, written) = self
                .resampler
                .process_into_buffer(&input, &mut output, Some(&indexing))
                .map_err(|error| error.to_string())?;
            let skip = self.delay.min(written);
            self.delay -= skip;
            let count = ((written - skip) as u128).min(remaining);
            self.frames_out += count;
            self.ready = skip * channels..(skip + count as usize) * channels;
        }
        Ok(self.ready.next().map(|i| self.output[i]))
    }
}
