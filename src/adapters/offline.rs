use crate::{
    adapters::{media_loader, telegram::Telegram},
    domain::Track,
};
use serde_json::json;
use std::{
    path::Path,
    sync::{atomic::AtomicU64, Mutex},
    time::Duration,
};
pub(crate) type Progress = Mutex<Option<(u64, String)>>;
/// A cancelled generation never publishes over another generation's file.
pub(crate) fn download(
    downloader: &Telegram,
    t: &Track,
    dir: &Path,
    cancel: &AtomicU64,
    generation: u64,
    progress: &Progress,
) -> Result<String, String> {
    let t = media_loader::resolve(downloader, t)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    downloader.call(json!({"@type":"downloadFile","file_id":t.file,"priority":8,"offset":0,"limit":0,"synchronous":false}))?;
    let mut complete = None;
    for _ in 0..6000 {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) != generation {
            return Err("Download cancelled".to_owned());
        }
        let v = downloader.call(json!({"@type":"getFile","file_id":t.file}))?;
        if v["local"]["is_downloading_completed"] == true {
            complete = Some(v);
            break;
        }
        *progress.lock().unwrap() = Some((
            generation,
            format!(
                "Offline: {} · {} / {} bytes",
                t.title, v["local"]["downloaded_size"], v["size"]
            ),
        ));
        std::thread::sleep(Duration::from_millis(100));
    }
    let v = complete.ok_or("Download timed out. Try saving again.".to_owned())?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) != generation {
        return Err("Download cancelled".into());
    }
    let path = dir.join(format!("{}-{}-{generation}", t.chat, t.message));
    let temp = path.with_extension("part");
    std::fs::copy(
        v["local"]["path"].as_str().ok_or("File unavailable")?,
        &temp,
    )
    .map_err(|e| e.to_string())?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) != generation {
        let _ = std::fs::remove_file(&temp);
        return Err("Download cancelled".into());
    }
    std::fs::rename(temp, &path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into())
}
