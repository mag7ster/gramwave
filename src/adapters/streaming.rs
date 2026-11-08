use crate::telegram::Telegram;
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
        .call_async(json!({"@type":"getFile","file_id":id}), &endpoint.cancel)
        .await
        .map_err(|e| e.to_string())?;
    let size = info["size"].as_u64().ok_or("Unknown file size")?;
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
    let transfer = Transfer {
        endpoint,
        id,
        position: start,
        end,
        _admission: admission,
        _active: active,
        lease: None,
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
    _admission: OwnedSemaphorePermit,
    _active: OwnedSemaphorePermit,
    lease: Option<super::telegram::DownloadLease>,
}
impl Transfer {
    async fn next(&mut self) -> Result<Vec<u8>, String> {
        let endpoint = &self.endpoint;
        if self.lease.is_none() {
            self.lease = Some(
                endpoint
                    .tg
                    .begin_download(self.id, self.position, false, &endpoint.cancel)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
        let read = async {
            loop {
                let prefix = endpoint.tg.call_async(json!({"@type":"getFileDownloadedPrefixSize","file_id":self.id,"offset":self.position}), &endpoint.cancel).await.map_err(|e| e.to_string())?["size"].as_u64().unwrap_or(0);
                if prefix == 0 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                let info = endpoint
                    .tg
                    .call_async(
                        json!({"@type":"getFile","file_id":self.id}),
                        &endpoint.cancel,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                let path = info["local"]["path"].as_str().ok_or("No local file")?;
                let mut file = tokio::fs::File::open(path)
                    .await
                    .map_err(|e| e.to_string())?;
                file.seek(io::SeekFrom::Start(self.position))
                    .await
                    .map_err(|e| e.to_string())?;
                let mut bytes =
                    vec![0; prefix.min(65536).min(self.end - self.position + 1) as usize];
                file.read_exact(&mut bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok::<_, String>(bytes);
            }
        };
        let bytes = tokio::select! {
            biased;
            _ = endpoint.cancel.cancelled() => return Err("Transfer cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(30), read) => result.map_err(|_| "Download stalled".to_owned())??,
        };
        self.position += bytes.len() as u64;
        Ok(bytes)
    }
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
