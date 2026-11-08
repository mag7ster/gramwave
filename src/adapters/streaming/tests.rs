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
    #[test]
    fn cached_range_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tg-player-range-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample");
        std::fs::write(&file, b"0123456789abcdef").unwrap();
        let (base, mut server) = serve(Telegram::mock_file(file)).unwrap();
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
        write!(socket, "GET /wrong/1 HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
        server.shutdown(Duration::from_secs(7)).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
