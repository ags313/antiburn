#[path = "../src/opencode_live.rs"]
mod opencode_live;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::AtomicBool,
    thread,
    time::Duration,
};

use opencode_live::{
    ProbeError, RegistrationError, ServiceRegistration, SseDecoder, SseError, decode_registration,
    default_registration_path, probe_registration, read_registration,
};

fn registration(url: &str) -> ServiceRegistration {
    let json = serde_json::json!({
        "id": "service-id",
        "version": "2.0.32",
        "url": url,
        "pid": 9123,
        "password": "private-token",
    });
    decode_registration(&serde_json::to_vec(&json).unwrap()).unwrap()
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).to_ascii_lowercase()
}

#[test]
fn accepts_only_local_http_registration_and_redacts_password_debug() {
    let local = registration("http://127.0.0.1:4400");
    let debug = format!("{local:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("private-token"));

    for url in [
        "https://127.0.0.1:4400",
        "http://example.com:4400",
        "http://127.0.0.1",
        "http://user@127.0.0.1:4400",
        "http://127.0.0.1:4400/?target=remote",
    ] {
        let value = serde_json::json!({"url": url, "pid": 10});
        assert_eq!(
            decode_registration(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
            RegistrationError::InvalidEndpoint,
            "{url}"
        );
    }
}

#[test]
fn rejects_malformed_and_invalid_registration_fields() {
    assert_eq!(decode_registration(b"{"), Err(RegistrationError::Malformed));
    assert_eq!(
        decode_registration(br#"{"url":"http://127.0.0.1:3","pid":0}"#),
        Err(RegistrationError::InvalidField)
    );
}

#[test]
fn reads_a_bounded_registration_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("service.json");
    std::fs::write(&path, br#"{"url":"http://127.0.0.1:4","pid":12}"#).unwrap();
    assert_eq!(read_registration(&path).unwrap().pid, 12);
    assert!(
        default_registration_path()
            .unwrap()
            .ends_with("opencode/service.json")
    );
    std::fs::write(&path, vec![b' '; 16 * 1024 + 1]).unwrap();
    assert_eq!(
        read_registration(&path).unwrap_err(),
        RegistrationError::TooLarge
    );
}

#[test]
fn local_probe_checks_pid_and_version() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let count = stream.read(&mut request).unwrap();
        let body = b"{\"pid\":9123,\"version\":\"2.0.32\"}";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        String::from_utf8_lossy(&request[..count]).to_ascii_lowercase()
    });
    let service = registration(&format!("http://{address}"));
    let info = probe_registration(&service, Duration::from_secs(2)).unwrap();
    assert_eq!(info.pid, 9123);
    assert_eq!(info.version, "2.0.32");
    let request = server.join().unwrap();
    assert!(request.contains("authorization: basic "));
    assert!(!request.contains("private-token"));
}

#[test]
fn probe_never_follows_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://example.com/api/info\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    let service = registration(&format!("http://{address}"));
    assert_eq!(
        probe_registration(&service, Duration::from_secs(2)).unwrap_err(),
        ProbeError::Redirect
    );
    server.join().unwrap();
}

#[test]
fn decodes_fragmented_json_crlf_comments_and_heartbeats() {
    let mut decoder = SseDecoder::new(1024);
    assert!(decoder.push(b": heartbeat\r").is_empty());
    assert!(decoder.push(b"\n\r\n: another\n\n").is_empty());
    assert!(decoder.push(b"event: test\r\nid: 7\r\nda").is_empty());
    let events = decoder.push(b"ta: {\"ok\":\r\ndata: true}\r\n\r");
    assert_eq!(events.len(), 1);
    let event = events[0].as_ref().unwrap();
    assert_eq!(event.event.as_deref(), Some("test"));
    assert_eq!(event.id.as_deref(), Some("7"));
    assert_eq!(event.data, serde_json::json!({"ok": true}));

    let mut utf8 = SseDecoder::new(128);
    assert!(utf8.push(b"data: {\"name\":\"caf\xc3").is_empty());
    let event = utf8.push(b"\xa9\"}\n\n");
    assert_eq!(
        event[0].as_ref().unwrap().data,
        serde_json::json!({"name": "café"})
    );
}

#[test]
fn reports_malformed_json_without_retaining_payload_in_error() {
    let mut decoder = SseDecoder::new(128);
    let events = decoder.push(b"data: private malformed payload\n\n");
    assert_eq!(events, vec![Err(SseError::MalformedJson)]);
}

#[test]
fn rejects_oversize_frames_and_resumes_after_the_frame_boundary() {
    let mut decoder = SseDecoder::new(12);
    let events = decoder.push(b"data: 123456789012345\n\ndata: 1\n\n");
    assert_eq!(events[0], Err(SseError::FrameTooLarge));
    assert_eq!(events[1].as_ref().unwrap().data, serde_json::json!(1));
}

#[test]
fn cancellation_clears_partial_data_and_stops_future_decoding() {
    let mut decoder = SseDecoder::new(128);
    assert!(decoder.push(b"data: {\"unfinished\"").is_empty());
    decoder.cancel();
    assert_eq!(decoder.push(b": 1}\n\n"), vec![Err(SseError::Cancelled)]);
}

#[test]
fn bounds_events_emitted_from_one_input_chunk() {
    let mut decoder = SseDecoder::new(32);
    let input = b"data: 1\n\n".repeat(65);
    let events = decoder.push(&input);
    assert_eq!(events.len(), 65);
    assert_eq!(events.last(), Some(&Err(SseError::TooManyEvents)));
    assert_eq!(decoder.push(b"data: 2\n\n"), vec![Err(SseError::Cancelled)]);
}

#[cfg(unix)]
#[test]
fn unix_binding_requires_the_registered_pid_to_hold_the_database_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("opencode.db");
    let _connection = rusqlite::Connection::open(&path).unwrap();

    assert!(opencode_live::unix::process_has_open_database(std::process::id(), &path).unwrap());

    let other = directory.path().join("other.db");
    drop(rusqlite::Connection::open(&other).unwrap());
    assert!(!opencode_live::unix::process_has_open_database(std::process::id(), &other).unwrap());
}

#[cfg(unix)]
#[test]
fn database_association_requires_stable_registration_and_file_use() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("opencode.db");
    let _connection = rusqlite::Connection::open(&database).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let pid = std::process::id();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).unwrap();
        let body = format!("{{\"pid\":{pid},\"version\":\"2.0.32\"}}");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let registration_path = directory.path().join("service.json");
    std::fs::write(
        &registration_path,
        serde_json::json!({
            "id":"service-id",
            "version":"2.0.32",
            "url":format!("http://{address}"),
            "pid":pid,
        })
        .to_string(),
    )
    .unwrap();

    let info = opencode_live::verify_database_association(
        &registration_path,
        &database,
        Duration::from_secs(2),
    )
    .unwrap();

    server.join().unwrap();
    assert_eq!(info.pid, pid);
}

#[cfg(unix)]
#[tokio::test]
async fn verified_event_stream_uses_only_the_bound_local_service() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("opencode.db");
    let _connection = rusqlite::Connection::open(&database).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let pid = std::process::id();
    let server = thread::spawn(move || {
        let mut event_request = String::new();
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            if request.starts_with("get /api/info ") {
                let body = format!("{{\"pid\":{pid},\"version\":\"2.0.32\"}}");
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            } else {
                event_request = request;
                let mut body = String::from(
                    "data: {\"id\":\"connected\",\"type\":\"server.connected\",\"data\":{}}\n\n: ",
                );
                body.push_str(&"x".repeat(16 * 1024 + 128));
                body.push_str(
                    "\n\n: heartbeat\n\ndata: {\"id\":\"after-heartbeat\",\"type\":\"session.updated\",\"data\":{\"sessionID\":\"ses_test\"}}\n\n",
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        }
        event_request
    });
    let registration_path = directory.path().join("service.json");
    std::fs::write(
        &registration_path,
        serde_json::json!({
            "id":"service-id",
            "version":"2.0.32",
            "url":format!("http://{address}"),
            "pid":pid,
            "password":"private-token",
        })
        .to_string(),
    )
    .unwrap();

    let mut stream = opencode_live::stream::open_verified_event_stream(
        &registration_path,
        &database,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let event = stream.next_event(&AtomicBool::new(false)).await.unwrap();
    let after_heartbeat = stream.next_event(&AtomicBool::new(false)).await.unwrap();

    assert_eq!(event.data["type"], "server.connected");
    assert_eq!(after_heartbeat.data["id"], "after-heartbeat");
    assert_eq!(stream.service_info().pid, pid);
    assert_eq!(stream.registration().pid, pid);
    let event_request = server.join().unwrap();
    assert!(event_request.starts_with("get /api/event "));
    assert!(event_request.contains("authorization: basic "));
    assert!(!event_request.contains("private-token"));
}

#[cfg(unix)]
#[tokio::test]
async fn verified_event_stream_rejects_redirects_without_following_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("opencode.db");
    let _connection = rusqlite::Connection::open(&database).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let pid = std::process::id();
    let server = thread::spawn(move || {
        let mut event_request = String::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            if request.starts_with("get /api/info ") {
                let body = format!("{{\"pid\":{pid},\"version\":\"2.0.32\"}}");
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            } else {
                event_request = request;
                stream
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
            }
        }
        event_request
    });
    let registration_path = directory.path().join("service.json");
    std::fs::write(
        &registration_path,
        serde_json::json!({
            "url":format!("http://{address}"),
            "pid":pid,
            "password":"private-token",
        })
        .to_string(),
    )
    .unwrap();

    let result = opencode_live::stream::open_verified_event_stream(
        &registration_path,
        &database,
        Duration::from_secs(2),
    )
    .await;

    assert!(matches!(
        result,
        Err(opencode_live::stream::EventStreamError::Redirect)
    ));
    let event_request = server.join().unwrap();
    assert!(event_request.starts_with("get /api/event "));
    assert!(event_request.contains("authorization: basic "));
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_event_stream_drops_the_open_response() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("opencode.db");
    let _connection = rusqlite::Connection::open(&database).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let pid = std::process::id();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            let request = read_http_request(&mut socket);
            if request.starts_with("get /api/info ") {
                let body = format!("{{\"pid\":{pid},\"version\":\"2.0.32\"}}");
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            } else {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n",
                    )
                    .unwrap();
            }
        }
    });
    let registration_path = directory.path().join("service.json");
    std::fs::write(
        &registration_path,
        serde_json::json!({
            "url":format!("http://{address}"),
            "pid":pid,
        })
        .to_string(),
    )
    .unwrap();

    let mut stream = opencode_live::stream::open_verified_event_stream(
        &registration_path,
        &database,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let error = stream.next_event(&AtomicBool::new(true)).await.unwrap_err();

    assert!(matches!(
        error,
        opencode_live::stream::EventStreamError::Cancelled
    ));
    server.join().unwrap();
}
