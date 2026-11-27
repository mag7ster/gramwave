use crate::telegram::{DownloadLease, DownloadState, Telegram};
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::json;
use std::{
    io,
    sync::{mpsc, Arc},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::sync::CancellationToken;

pub(super) const PREFETCH_HEADER: &str = "x-gramwave-prefetch";

pub struct Server {
    cancel: CancellationToken,
    done: Option<mpsc::Receiver<Result<(), String>>>,
}
impl Server {
    pub fn stop(&self) {
        self.cancel.cancel();
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.stop();
        let Some(done) = self.done.take() else {
            return Ok(());
        };
        done.recv_timeout(timeout)
            .map_err(|e| format!("HTTP shutdown: {e}"))?
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Clone)]
struct Endpoint {
    tg: Telegram,
    token: String,
    cancel: CancellationToken,
    active: Arc<Semaphore>,
    admission: Arc<Semaphore>,
}
/// A loopback endpoint running on the desktop's shared runtime.
pub fn serve(tg: Telegram, runtime: tokio::runtime::Handle) -> Result<(String, Server), String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).map_err(|e| e.to_string())?;
    let token: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let base = format!("http://127.0.0.1:{port}/{token}");
    let cancel = CancellationToken::new();
    let endpoint = Endpoint {
        tg,
        token,
        cancel: cancel.clone(),
        active: Arc::new(Semaphore::new(4)),
        admission: Arc::new(Semaphore::new(20)),
    };
    let router = Router::new()
        .route("/{token}/{file}", get(respond))
        .with_state(endpoint);
    let (tx, done) = mpsc::channel();
    let stop = cancel.clone();
    runtime.spawn(async move {
        let result = async {
            let listener =
                tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
            axum::serve(listener, router)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        let _ = tx.send(result);
    });
    Ok((
        base,
        Server {
            cancel,
            done: Some(done),
        },
    ))
}
fn status_response(status: StatusCode) -> Response {
    let mut response = status.into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, "close".parse().unwrap());
    response
}
async fn respond(
    State(endpoint): State<Endpoint>,
    Path((token, id)): Path<(String, i64)>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if token != endpoint.token || id <= 0 {
        return status_response(StatusCode::NOT_FOUND);
    }
    let Ok(admission) = endpoint.admission.clone().try_acquire_owned() else {
        return status_response(StatusCode::SERVICE_UNAVAILABLE);
    };
    let active = tokio::select! {
        biased;
        _ = endpoint.cancel.cancelled() => return status_response(StatusCode::SERVICE_UNAVAILABLE),
        permit = endpoint.active.clone().acquire_owned() => match permit { Ok(p) => p, Err(_) => return status_response(StatusCode::SERVICE_UNAVAILABLE) },
    };
    match response(endpoint, id, method, headers, admission, active).await {
        Ok(response) => response,
        Err(_) => status_response(StatusCode::BAD_GATEWAY),
    }
}
async fn response(
    endpoint: Endpoint,
    id: i64,
    method: Method,
    headers: HeaderMap,
    admission: OwnedSemaphorePermit,
    active: OwnedSemaphorePermit,
) -> Result<Response, String> {
    let info = endpoint
        .tg
        .get_file(id, &endpoint.cancel)
        .await
        .map_err(|e| e.to_string())?;
    let size = info.size.ok_or("Unknown file size")?;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    // Multiple ranges are deliberately ignored, including overlapping ranges.
    let range = range.filter(|v| !v.contains(','));
    let parsed = parse_range(range.map(|v| v.strip_prefix("bytes=").unwrap_or(v)), size);
    let (start, end) = match parsed {
        Ok(r) => r,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .header(header::CONNECTION, "close")
                .header(header::CONTENT_LENGTH, "0")
                .body(Body::empty())
                .unwrap())
        }
    };
    let mut builder = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, (end - start + 1).to_string())
        .header(header::CONNECTION, "close");
    if range.is_some() {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    if method == Method::HEAD {
        return Ok(builder.body(Body::empty()).unwrap());
    }
    let speculative = headers
        .get(PREFETCH_HEADER)
        .is_some_and(|value| value == "1");
    let transfer = Transfer {
        endpoint,
        id,
        position: start,
        end,
        speculative,
        _admission: admission,
        _active: active,
        lease: None,
        file: None,
        confirmed: None,
    };
    let stream = futures_util::stream::try_unfold(transfer, |mut transfer| async move {
        if transfer.position > transfer.end {
            return Ok::<_, io::Error>(None);
        }
        let bytes = transfer.next().await.map_err(io::Error::other)?;
        Ok(Some((Bytes::from(bytes), transfer)))
    });
    Ok(builder.body(Body::from_stream(stream)).unwrap())
}
struct Transfer {
    endpoint: Endpoint,
    id: i64,
    position: u64,
    end: u64,
    speculative: bool,
    _admission: OwnedSemaphorePermit,
    _active: OwnedSemaphorePermit,
    lease: Option<DownloadLease>,
    file: Option<(String, tokio::fs::File)>,
    confirmed: Option<(DownloadState, u64)>,
}

impl Transfer {
    async fn next(&mut self) -> Result<Vec<u8>, String> {
        let cancel = self.endpoint.cancel.clone();
        let bytes = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("Transfer cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(30), self.read()) => result.map_err(|_| "Download stalled".to_owned())??,
        };
        self.position += bytes.len() as u64;
        Ok(bytes)
    }
    async fn read(&mut self) -> Result<Vec<u8>, String> {
        let Self {
            ref endpoint,
            id,
            position,
            end,
            speculative,
            ..
        } = *self;
        let lease = match &mut self.lease {
            Some(lease) => lease,
            slot @ None => slot.insert(
                endpoint
                    .tg
                    .begin_stream_download(
                        id,
                        position,
                        if speculative { end - position + 1 } else { 0 },
                        &endpoint.cancel,
                    )
                    .await
                    .map_err(|e| e.to_string())?,
            ),
        };
        let mut refresh_path = true;
        let mut check = tokio::time::interval(Duration::from_millis(100));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let state = lease.state.borrow_and_update().clone();
            let prefix = if state.complete {
                end - position + 1
            } else {
                let known = if state.offset <= position {
                    state.available_until
                } else {
                    0
                };
                let confirmed = self
                    .confirmed
                    .as_ref()
                    .filter(|(source, _)| source == &state)
                    .map_or(0, |(_, end)| *end);
                known.max(confirmed).saturating_sub(position)
            };
            if let Some(path) = state.path.as_deref().filter(|_| prefix > 0) {
                let mut bytes = vec![0; prefix.min(65536).min(end - position + 1) as usize];
                match read_chunk(&mut self.file, path, position, &mut bytes).await {
                    Ok(()) => return Ok(bytes),
                    Err(e)
                        if refresh_path
                            && matches!(
                                e.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::UnexpectedEof
                            ) =>
                    {
                        refresh_path = false;
                        self.file = None;
                        self.confirmed = None;
                        endpoint
                            .tg
                            .get_file(id, &endpoint.cancel)
                            .await
                            .map_err(|e| e.to_string())?;
                        continue;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            tokio::select! {
                biased;
                _ = check.tick() => {
                    let prefix = endpoint.tg.call_async(json!({"@type":"getFileDownloadedPrefixSize","file_id":id,"offset":position}), &endpoint.cancel).await.map_err(|e| e.to_string())?["size"].as_u64().unwrap_or_default();
                    if prefix > 0 {
                        let latest = lease.state.borrow().clone();
                        if latest.path.is_none() {
                            endpoint.tg.get_file(id, &endpoint.cancel).await.map_err(|e| e.to_string())?;
                        } else if latest == state {
                            self.confirmed = Some((state, position.saturating_add(prefix)));
                        }
                    }
                }
                changed = lease.state.changed() => changed.map_err(|_| "Download closed".to_owned())?,
            }
        }
    }
}

async fn read_chunk(
    open: &mut Option<(String, tokio::fs::File)>,
    path: &str,
    position: u64,
    bytes: &mut [u8],
) -> io::Result<()> {
    if open.as_ref().is_none_or(|(current, _)| current != path) {
        let mut file = tokio::fs::File::open(path).await?;
        file.seek(io::SeekFrom::Start(position)).await?;
        *open = Some((path.into(), file));
    }
    open.as_mut().unwrap().1.read_exact(bytes).await?;
    Ok(())
}
fn parse_range(range: Option<&str>, size: u64) -> Result<(u64, u64), String> {
    if size == 0 {
        return Err("Empty resource".into());
    }
    let Some(range) = range else {
        return Ok((0, size - 1));
    };
    let ranges = http_range_header::parse_range_header(&format!("bytes={range}"))
        .map_err(|e| e.to_string())?
        .validate(size)
        .map_err(|e| e.to_string())?;
    if ranges.len() != 1 {
        return Err("Multiple ranges".into());
    }
    Ok((*ranges[0].start(), *ranges[0].end()))
}
#[cfg(test)]
#[path = "streaming/tests.rs"]
mod tests;
