#[path = "../src/opencode_live.rs"]
mod opencode_live;

use std::{
    io::{Read, Write},
    net::TcpListener,
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
