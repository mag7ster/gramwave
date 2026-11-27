use super::audio_source::{HttpSource, LocalSource};
use std::{
    fs::File,
    path::Path,
    sync::{Arc, OnceLock},
};
use symphonia::core::{
    codecs::{
        audio::{well_known::CODEC_ID_MP3, AudioDecoder},
        registry::CodecRegistry,
        CodecParameters,
    },
    errors::{Error, Result},
    formats::{probe::Hint, FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSource, MediaSourceStream},
    units::{Time, TimeBase},
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

pub(super) struct Decode {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    time_base: TimeBase,
    pub(super) duration: f64,
    coarse_seek: bool,
}

pub(super) struct Pcm {
    pub(super) position: f64,
    pub(super) channels: u16,
    pub(super) rate: u32,
}

impl Decode {
    pub(super) fn open(
        path: &str,
        file: Option<Arc<File>>,
        runtime: Handle,
        cancel: CancellationToken,
        promote: Option<CancellationToken>,
    ) -> Result<Self> {
        let mut hint = Hint::new();
        let mut cbr = false;
        let source: Box<dyn MediaSource> = if path.starts_with("http://") {
            let mut source = HttpSource::open(path, runtime, cancel, promote)?;
            let mut prefix = [0; 8192];
            let count = source.peek_prefix(&mut prefix)?;
            cbr = declares_constant_mp3_bitrate(&prefix[..count]);
            Box::new(source)
        } else {
            if let Some(extension) = Path::new(path).extension().and_then(|ext| ext.to_str()) {
                hint.with_extension(extension);
            }
            Box::new(LocalSource::new(match file {
                Some(file) => file,
                None => Arc::new(File::open(path)?),
            })?)
        };
        let stream = MediaSourceStream::new(source, Default::default());
        let format = symphonia::default::get_probe().probe(
            &hint,
            stream,
            FormatOptions::default(),
            Default::default(),
        )?;
        static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
        let codecs = CODECS.get_or_init(|| {
            let mut codecs = CodecRegistry::new();
            symphonia::default::register_enabled_codecs(&mut codecs);
            codecs.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
            codecs
        });
        for track in format.tracks() {
            let (Some(CodecParameters::Audio(params)), Some(time_base)) =
                (&track.codec_params, track.time_base)
            else {
                continue;
            };
            if let Ok(decoder) = codecs.make_audio_decoder(params, &Default::default()) {
                let duration = track
                    .duration
                    .and_then(|duration| time_base.calc_duration(duration))
                    .map_or_else(
                        || {
                            track
                                .num_frames
                                .zip(params.sample_rate)
                                .map_or(0., |(frames, rate)| frames as f64 / f64::from(rate))
                        },
                        |time| time.as_secs_f64(),
                    );
                let id = track.id;
                let coarse_seek = cbr && params.codec == CODEC_ID_MP3 && duration > 0.;
                return Ok(Self {
                    format,
                    decoder,
                    track: id,
                    time_base,
                    duration,
                    coarse_seek,
                });
            }
        }
        Err(Error::Unsupported("No supported audio track"))
    }

    pub(super) fn seek(&mut self, seconds: f64) -> Result<()> {
        let time =
            Time::try_from_secs_f64(seconds).ok_or(Error::Unsupported("Invalid seek time"))?;
        let mode = if self.coarse_seek {
            SeekMode::Coarse
        } else {
            SeekMode::Accurate
        };
        self.format.seek(
            mode,
            SeekTo::Time {
                time,
                track_id: Some(self.track),
            },
        )?;
        self.decoder.reset();
        Ok(())
    }

    pub(super) fn next(&mut self, after: f64, samples: &mut Vec<f32>) -> Result<Option<Pcm>> {
        while let Some(packet) = self.format.next_packet()? {
            if packet.track_id != self.track {
                continue;
            }
            let decoded = self.decoder.decode(&packet)?;
            let channels = decoded.spec().channels().count();
            let rate = decoded.spec().rate();
            let position = self
                .time_base
                .calc_time_saturating(packet.pts.saturating_add(packet.trim_start))
                .as_secs_f64();
            let skip = if after > 0. {
                ((after - position) * f64::from(rate)).round().max(0.) as usize
            } else {
                0
            };
            let frames = decoded.frames();
            if skip >= frames {
                continue;
            }
            samples.resize(frames * channels, 0.);
            decoded.copy_to_slice_interleaved(samples.as_mut_slice());
            samples.drain(..skip * channels);
            return Ok(Some(Pcm {
                channels: channels as u16,
                rate,
                position: (position + skip as f64 / f64::from(rate)).max(0.),
            }));
        }
        Ok(None)
    }
}

fn declares_constant_mp3_bitrate(prefix: &[u8]) -> bool {
    let mut frame = prefix;
    while frame.starts_with(b"ID3") {
        let Some(h) = frame.get(..10) else {
            return false;
        };
        if h[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return false;
        }
        let size = h[6..10]
            .iter()
            .fold(0_usize, |size, byte| (size << 7) | usize::from(*byte));
        let footer = if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
        let Some(next) = frame.get(10 + size + footer..) else {
            return false;
        };
        frame = next;
    }
    let Some(&[a, b, c, d]) = frame.first_chunk::<4>() else {
        return false;
    };
    let version = (b >> 3) & 3;
    let bitrate = c >> 4;
    if a != 0xff
        || b & 0xe6 != 0xe2
        || version == 1
        || !(1..15).contains(&bitrate)
        || c & 0x0c == 0x0c
    {
        return false;
    }
    let mono = d >> 6 == 3;
    let side_info = match (version == 3, mono) {
        (true, false) => 32,
        (false, true) => 9,
        _ => 17,
    };
    let offset = 4 + side_info;
    frame.get(offset..offset + 4) == Some(b"Info")
        && frame
            .get(offset + 4..offset + 16)
            .is_some_and(|info| info[3] & 3 == 3 && info[4..8] != [0; 4] && info[8..12] != [0; 4])
}

#[cfg(all(test, feature = "native-tests"))]
mod tests;
