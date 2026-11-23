use super::{telegram::Telegram, telegram_mapping::telegram_int64};
use eframe::egui::{Color32, ColorImage};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use std::{io::Read, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const SIZE: usize = 64;
const FPS: u32 = 24;
const MAX_FRAMES: usize = FPS as usize * 3;
const MAX_FILE: usize = 512 * 1024;
const MAX_JSON: usize = 1024 * 1024;
pub(crate) const DEMO_ID: i64 = 1;
const DEMO_TGS: &[u8] = br#"{
  "v":"5.7.0","w":64,"h":64,"fr":24,"ip":0,"op":24,
  "layers":[{"ty":4,"ind":1,"ip":0,"op":24,"st":0,
    "ks":{"p":{"a":0,"k":[32,32,0]},"s":{"a":1,"k":[
      {"t":0,"s":[62,62,100],"e":[100,100,100],"i":{"x":0.6,"y":1},"o":{"x":0.4,"y":0}},
      {"t":12,"s":[100,100,100],"e":[62,62,100],"i":{"x":0.6,"y":1},"o":{"x":0.4,"y":0}},
      {"t":24,"s":[62,62,100]}]},"o":{"a":0,"k":100}},
    "shapes":[{"ty":"sh","ks":{"a":0,"k":{"v":[[0,-27],[7,-7],[27,0],[7,7],[0,27],[-7,7],[-27,0],[-7,-7]],
      "i":[[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]],"c":true}}},
      {"ty":"fl","c":{"a":0,"k":[1,0.72,0.22,1]},"o":{"a":0,"k":100},"r":1}]}]
}"#;

pub(crate) struct Animation {
    pub frames: Vec<ColorImage>,
    pub duration: Duration,
}

impl Animation {
    fn still(frame: ColorImage) -> Self {
        Self {
            frames: vec![frame],
            duration: Duration::ZERO,
        }
    }
}

pub(crate) struct EmojiAsset {
    pub fallback: String,
    pub adaptive: bool,
    pub animation: Option<Animation>,
}

pub(crate) fn demo_asset() -> EmojiAsset {
    EmojiAsset {
        fallback: "✨".into(),
        adaptive: false,
        animation: tgs_frames(DEMO_TGS, &CancellationToken::new()),
    }
}

pub(crate) async fn fetch(
    telegram: &Telegram,
    ids: &[i64],
    cancel: &CancellationToken,
) -> Vec<(i64, EmojiAsset)> {
    let request_ids = ids.iter().map(i64::to_string).collect::<Vec<_>>();
    let Ok(reply) = telegram
        .call_async(
            json!({"@type":"getCustomEmojiStickers", "custom_emoji_ids":request_ids}),
            cancel,
        )
        .await
    else {
        return Vec::new();
    };
    let stickers = reply["stickers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|sticker| {
            let kind = &sticker["full_type"];
            let id = telegram_int64(&kind["custom_emoji_id"])?;
            (kind["@type"] == "stickerFullTypeCustomEmoji" && ids.contains(&id))
                .then(|| (id, sticker.clone()))
        })
        .collect::<Vec<_>>();
    stream::iter(stickers)
        .map(|(id, sticker)| async move {
            let mut animation =
                load(telegram, &sticker["sticker"], &sticker["format"], cancel).await;
            if animation.is_none() && !cancel.is_cancelled() {
                animation = load(
                    telegram,
                    &sticker["thumbnail"]["file"],
                    &Value::Null,
                    cancel,
                )
                .await;
            }
            let adaptive = sticker["full_type"]["needs_repainting"] == true;
            if let Some(animation) = animation.as_mut().filter(|_| adaptive) {
                for picture in &mut animation.frames {
                    for pixel in &mut picture.pixels {
                        *pixel = Color32::from_white_alpha(pixel.a());
                    }
                }
            }
            (
                id,
                EmojiAsset {
                    fallback: sticker["emoji"].as_str().unwrap_or_default().into(),
                    adaptive,
                    animation,
                },
            )
        })
        .buffer_unordered(2)
        .collect()
        .await
}

async fn load(
    telegram: &Telegram,
    file: &Value,
    format: &Value,
    cancel: &CancellationToken,
) -> Option<Animation> {
    let id = file["id"].as_i64().filter(|&id| id > 0)?;
    if file["size"]
        .as_u64()
        .is_some_and(|size| size > MAX_FILE as u64)
    {
        return None;
    }
    let (path, cache) = telegram.cached_artwork(id, cancel).await.ok()?;
    let mut raw = Vec::new();
    let mut file = tokio::fs::File::open(path)
        .await
        .ok()?
        .take((MAX_FILE + 1) as u64);
    tokio::select! {
        _ = cancel.cancelled() => return None,
        result = file.read_to_end(&mut raw) => result.ok()?,
    };
    drop(cache);
    if raw.is_empty() || raw.len() > MAX_FILE {
        return None;
    }
    if format["@type"] == "stickerFormatWebm" {
        return webm(&raw, cancel).await;
    }
    let tgs = format["@type"] == "stickerFormatTgs";
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        if tgs {
            let mut json = Vec::new();
            flate2::read::GzDecoder::new(&raw[..])
                .take((MAX_JSON + 1) as u64)
                .read_to_end(&mut json)
                .ok()?;
            tgs_frames(&json, &cancel)
        } else {
            static_frame(&raw)
        }
    })
    .await
    .ok()?
}

fn static_frame(raw: &[u8]) -> Option<Animation> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(raw))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .ok()?
        .resize_exact(
            SIZE as u32,
            SIZE as u32,
            image::imageops::FilterType::Triangle,
        )
        .into_rgba8();
    Some(Animation::still(ColorImage::from_rgba_unmultiplied(
        [SIZE; 2], &rgba,
    )))
}

pub(crate) fn tgs_frames(json: &[u8], cancel: &CancellationToken) -> Option<Animation> {
    let composition = tlottie::Composition::parse(
        json,
        &tlottie::Limits {
            max_input_bytes: MAX_JSON,
            max_dimension: 512,
            ..Default::default()
        },
    )
    .ok()?;
    let duration = composition.duration_secs();
    if !duration.is_finite() || duration <= 0. || duration > 3.1 {
        return None;
    }
    let count = if composition.is_static() {
        1
    } else {
        ((duration * FPS as f32).ceil() as usize).min(MAX_FRAMES)
    };
    let original_frames = composition.frame_count() as f32;
    let mut renderer = tlottie::CPURenderer::new(composition);
    let mut words = vec![0u32; SIZE * SIZE];
    let mut frames = Vec::with_capacity(count);
    for index in 0..count {
        if cancel.is_cancelled() {
            return None;
        }
        renderer
            .render(
                index as f32 * original_frames / count as f32,
                &mut words,
                SIZE as u32,
                SIZE as u32,
                tlottie::RenderOptions::default(),
            )
            .ok()?;
        let raw: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        frames.push(ColorImage::from_rgba_premultiplied([SIZE; 2], &raw));
    }
    Some(Animation {
        frames,
        duration: Duration::from_secs_f32(duration),
    })
}

async fn webm(raw: &[u8], cancel: &CancellationToken) -> Option<Animation> {
    let filter = format!("fps={FPS},scale={SIZE}:{SIZE}:force_original_aspect_ratio=decrease,format=rgba,pad={SIZE}:{SIZE}:(ow-iw)/2:(oh-ih)/2:color=black@0");
    let mut ffmpeg = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-max_alloc",
            "16777216",
            "-filter_threads",
            "1",
            "-threads",
            "1",
            "-max_pixels",
            "262144",
            "-protocol_whitelist",
            "pipe",
            "-f",
            "matroska",
            "-c:v",
            "libvpx-vp9",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &filter,
            "-frames:v",
            &MAX_FRAMES.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut input = ffmpeg.stdin.take()?;
    let frame_bytes = SIZE * SIZE * 4;
    let max = MAX_FRAMES * frame_bytes;
    let mut output = ffmpeg.stdout.take()?.take((max + 1) as u64);
    let mut bytes = Vec::new();
    let decode = async {
        tokio::try_join!(
            async move { input.write_all(raw).await },
            output.read_to_end(&mut bytes),
        )
        .ok()?;
        if bytes.is_empty()
            || bytes.len() > max
            || bytes.len() % frame_bytes != 0
            || !ffmpeg.wait().await.ok()?.success()
        {
            return None;
        }
        let frames: Vec<_> = bytes
            .chunks_exact(frame_bytes)
            .map(|frame| ColorImage::from_rgba_unmultiplied([SIZE; 2], frame))
            .collect();
        let duration = Duration::from_secs_f32(frames.len() as f32 / FPS as f32);
        Some(Animation { frames, duration })
    };
    tokio::select! {
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(5), decode) => result.ok().flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animated_tgs_produces_distinct_transparent_frames() {
        let animation = demo_asset().animation.unwrap();
        assert_eq!(animation.duration, Duration::from_secs(1));
        assert_eq!(animation.frames.len(), FPS as usize);
        assert_ne!(animation.frames[0], animation.frames[FPS as usize / 2]);
        assert_eq!(animation.frames[0].pixels[0], Color32::TRANSPARENT);
        assert!(animation.frames[0]
            .pixels
            .iter()
            .any(|pixel| pixel.a() == 255));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(tgs_frames(DEMO_TGS, &cancel).is_none());
    }

    #[cfg(feature = "native-tests")]
    #[tokio::test]
    async fn animated_webm_preserves_transparency() {
        let raw = tokio::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i",
                   "color=c=red@0.2:s=100x100:r=24:d=0.25,format=yuva420p,drawbox=x=10:y=10:w=45:h=45:color=blue:t=fill:replace=1",
                   "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-f", "webm", "pipe:1"])
            .output().await.unwrap();
        assert!(raw.status.success());
        let animation = webm(&raw.stdout, &CancellationToken::new()).await.unwrap();
        assert!(animation.frames.len() > 1);
        assert!(animation
            .frames
            .iter()
            .flat_map(|frame| &frame.pixels)
            .any(|pixel| pixel.a() < 250));
    }
}
