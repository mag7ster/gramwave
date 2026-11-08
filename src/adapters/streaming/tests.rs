use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn byte_ranges() {
        assert_eq!(parse_range(None, 100), Ok((0, 99)));
        assert_eq!(parse_range(Some("30-"), 100), Ok((30, 99)));
        assert_eq!(parse_range(Some("-20"), 100), Ok((80, 99)));
        assert_eq!(parse_range(Some("20-300"), 100), Ok((20, 99)));
        for r in ["100-", "9-3", "0-1,4-5", "-0", "a-"] {
            assert!(parse_range(Some(r), 100).is_err());
        }
    }
}

#[cfg(all(test, feature = "native-tests"))]
mod streaming_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
    };
    #[test]
    fn cached_range_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let file = dir.join("sample");
        std::fs::write(&file, b"0123456789abcdef").unwrap();
        let workers = crate::application::tasks::Scheduler::new();
        let (base, mut server) = serve(Telegram::mock_file(file), workers.handle()).unwrap();
        let authority = base.strip_prefix("http://").unwrap();
        let (host, path) = authority.split_once('/').unwrap();
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET /{path}/1 HTTP/1.1\r\nHost: {host}\r\nRange: bytes=4-8\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 206"));
        assert!(response.ends_with("45678"));
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket, "GET /wrong/1 HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
        server.shutdown(Duration::from_secs(7)).unwrap();
    }
}

#[test]
fn head_ranges_permits_and_cancellation_follow_body_lifetime() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("media");
    std::fs::write(&path, b"0123456789abcdef").unwrap();
    let downloads = Arc::new(AtomicUsize::new(0));
    let count = downloads.clone();
    let tg = Telegram::mock_rpc(move |v| {
        let id = v["file_id"].as_i64().unwrap_or(1);
        if v["@type"] == "downloadFile" {
            count.fetch_add(1, Ordering::Relaxed);
        }
        if v["@type"] == "getFileDownloadedPrefixSize" {
            json!({"size":16-v["offset"].as_u64().unwrap()})
        } else {
            json!({"id":id,"size":16,"local":{"path":path,"is_downloading_completed":true}})
        }
    });
    let endpoint = Endpoint {
        tg,
        token: "secret".into(),
        cancel: Default::default(),
        active: Arc::new(Semaphore::new(4)),
        admission: Arc::new(Semaphore::new(20)),
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        async fn request(endpoint: Endpoint, method: Method, range: Option<&str>) -> Response {
            let mut headers = HeaderMap::new();
            if let Some(range) = range {
                headers.insert(header::RANGE, range.parse().unwrap());
            }
            let active = endpoint.active.clone().acquire_owned().await.unwrap();
            let admission = endpoint.admission.clone().acquire_owned().await.unwrap();
            response(endpoint, 1, method, headers, admission, active)
                .await
                .unwrap()
        }
        let head = request(endpoint.clone(), Method::HEAD, None).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "16");
        assert!(axum::body::to_bytes(head.into_body(), 32)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(downloads.load(Ordering::Relaxed), 0);
        let invalid = request(endpoint.clone(), Method::GET, Some("bytes=16-")).await;
        assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(invalid.headers()[header::CONTENT_RANGE], "bytes */16");
        let full = request(endpoint.clone(), Method::GET, Some("bytes=0-1,4-5")).await;
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(endpoint.active.available_permits(), 3);
        assert_eq!(
            axum::body::to_bytes(full.into_body(), 32).await.unwrap(),
            &b"0123456789abcdef"[..]
        );
        assert_eq!(endpoint.active.available_permits(), 4);
        let range = request(endpoint.clone(), Method::GET, Some("bytes=4-8")).await;
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            axum::body::to_bytes(range.into_body(), 32).await.unwrap(),
            &b"45678"[..]
        );
        let body = request(endpoint.clone(), Method::GET, None).await;
        endpoint.cancel.cancel();
        assert!(axum::body::to_bytes(body.into_body(), 32).await.is_err());
        assert_eq!(endpoint.active.available_permits(), 4);
    });
}
