use http_body_util::{BodyExt, Empty};
use hyper::{
    body::{Bytes, Incoming},
    client::conn::http1,
    header, Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use std::{
    io::{self, Read, Seek, SeekFrom},
    time::Duration,
};
use symphonia::core::io::MediaSource;
use tokio::{net::TcpStream, runtime::Handle};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

pub(super) struct HttpSource {
    path: String,
    port: u16,
    size: u64,
    offset: u64,
    runtime: Handle,
    cancel: CancellationToken,
    transfer: Option<Transfer>,
    promote: Option<CancellationToken>,
    seekable: bool,
}

struct Transfer {
    response: Response<Incoming>,
    _connection: AbortOnDropHandle<()>,
    bytes: Bytes,
    end: u64,
    speculative: bool,
}

impl HttpSource {
    pub(super) fn open(
        url: &str,
        runtime: Handle,
        cancel: CancellationToken,
        promote: Option<CancellationToken>,
    ) -> io::Result<Self> {
        let url = url::Url::parse(url).map_err(io::Error::other)?;
        let port = url
            .port()
            .filter(|_| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
            })
            .ok_or_else(|| invalid("Invalid loopback media URL"))?;
        let mut source = Self {
            path: url.path().into(),
            port,
            size: 0,
            offset: 0,
            runtime,
            cancel,
            transfer: None,
            promote,
            seekable: true,
        };
        let head = source.request(Method::HEAD)?;
        if head.response.status() != StatusCode::OK {
            return Err(invalid("Loopback media HEAD failed"));
        }
        source.size = head
            .response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse().ok())
            .filter(|&size| size > 0)
            .ok_or_else(|| invalid("Invalid loopback media length"))?;
        Ok(source)
    }

    pub(super) fn peek_prefix(&mut self, prefix: &mut [u8]) -> io::Result<usize> {
        debug_assert_eq!(self.offset, 0);
        let mut count = self.read(prefix)?;
        let marker = prefix.len().min(4).min(self.size as usize);
        if count < marker {
            self.read_exact(&mut prefix[count..marker])?;
            count = marker;
        }
        if let Some(transfer) = &mut self.transfer {
            let mut buffered = Vec::with_capacity(count + transfer.bytes.len());
            buffered.extend_from_slice(&prefix[..count]);
            buffered.extend_from_slice(&transfer.bytes);
            transfer.bytes = buffered.into();
        }
        self.offset = 0;
        if self.promote.is_some() && prefix[..count].starts_with(b"OggS") {
            self.seekable = false;
        }
        Ok(count)
    }

    fn request(&self, method: Method) -> io::Result<Transfer> {
        let speculative = self
            .promote
            .as_ref()
            .is_some_and(|token| !token.is_cancelled());
        let end = if speculative {
            self.size
                .saturating_sub(1)
                .min(self.offset.saturating_add(1_048_575))
        } else {
            self.size.saturating_sub(1)
        };
        self.runtime.block_on(async {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => Err(cancelled()),
                result = tokio::time::timeout(Duration::from_secs(65), async {
                    let socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port)).await?;
                    let (mut sender, connection) = http1::handshake(TokioIo::new(socket)).await.map_err(io::Error::other)?;
                    let task = AbortOnDropHandle::new(tokio::spawn(async move { let _ = connection.await; }));
                    let mut request = Request::builder()
                        .method(method.clone())
                        .uri(&self.path)
                        .header(header::HOST, format!("127.0.0.1:{}", self.port))
                        .header(header::CONNECTION, "close");
                    if method == Method::GET {
                        request = request.header(header::RANGE, format!("bytes={}-{end}", self.offset));
                        if speculative {
                            request = request.header(super::streaming::PREFETCH_HEADER, "1");
                        }
                    }
                    let response = sender
                        .send_request(request.body(Empty::<Bytes>::new()).map_err(io::Error::other)?)
                        .await
                        .map_err(io::Error::other)?;
                    Ok(Transfer { response, _connection: task, bytes: Bytes::new(), end, speculative })
                }) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Loopback media headers timed out"))?,
            }
        })
    }
}

impl Read for HttpSource {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(cancelled());
        }
        if destination.is_empty() || self.offset >= self.size {
            return Ok(0);
        }
        loop {
            if self.transfer.as_ref().is_some_and(|transfer| {
                transfer.end < self.offset
                    || (transfer.speculative
                        && self
                            .promote
                            .as_ref()
                            .is_some_and(CancellationToken::is_cancelled))
            }) {
                self.transfer = None;
            }
            if self.transfer.is_none() {
                let transfer = self.request(Method::GET)?;
                let expected = format!("bytes {}-{}/{}", self.offset, transfer.end, self.size);
                if transfer.response.status() != StatusCode::PARTIAL_CONTENT
                    || transfer
                        .response
                        .headers()
                        .get(header::CONTENT_RANGE)
                        .is_none_or(|value| value.as_bytes() != expected.as_bytes())
                {
                    return Err(invalid("Invalid loopback media range"));
                }
                self.transfer = Some(transfer);
            }
            let transfer = self.transfer.as_mut().unwrap();
            if transfer.bytes.is_empty() {
                let promote = self.promote.as_ref();
                let frame = self.runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = self.cancel.cancelled() => Err(cancelled()),
                        _ = async { if let Some(promote) = promote { promote.cancelled().await } }, if transfer.speculative => Ok(None),
                        result = transfer.response.body_mut().frame() => result
                            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "Incomplete loopback range"))?
                            .map(Some).map_err(io::Error::other),
                    }
                })?;
                if let Some(frame) = frame {
                    transfer.bytes = frame.into_data().unwrap_or_default();
                }
                continue;
            }
            let count = transfer
                .bytes
                .len()
                .min(destination.len())
                .min(usize::try_from(transfer.end + 1 - self.offset).unwrap_or(usize::MAX));
            destination[..count].copy_from_slice(&transfer.bytes.split_to(count));
            self.offset += count as u64;
            return Ok(count);
        }
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let offset = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.offset.checked_add_signed(delta),
            SeekFrom::End(delta) => self.size.checked_add_signed(delta),
        }
        .ok_or_else(|| invalid("Invalid media offset"))?;
        if offset != self.offset {
            self.transfer = None;
            self.offset = offset;
        }
        Ok(offset)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        self.seekable
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.size)
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn cancelled() -> io::Error {
    io::Error::other("Loopback media read cancelled")
}

pub(super) struct LocalSource {
    file: std::sync::Arc<std::fs::File>,
    offset: u64,
    size: u64,
}

impl LocalSource {
    pub(super) fn new(file: std::sync::Arc<std::fs::File>) -> io::Result<Self> {
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            offset: 0,
            size,
        })
    }
}

impl Read for LocalSource {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        use std::os::unix::fs::FileExt;
        let count = self.file.read_at(destination, self.offset)?;
        self.offset += count as u64;
        Ok(count)
    }
}

impl Seek for LocalSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.offset = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.offset.checked_add_signed(delta),
            SeekFrom::End(delta) => self.size.checked_add_signed(delta),
        }
        .ok_or_else(|| invalid("Invalid media offset"))?;
        Ok(self.offset)
    }
}

impl MediaSource for LocalSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.size)
    }
}
