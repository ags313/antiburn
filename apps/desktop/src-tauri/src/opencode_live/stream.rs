//! Opens and reads the bounded OpenCode V2 event stream.

use std::{
    collections::VecDeque,
    fmt,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

use reqwest::{Client, Response, redirect::Policy};

use super::{
    DatabaseAssociationError, ProbeError, RegistrationError, ServiceInfo, ServiceRegistration,
    SseDecoder, SseError, SseEvent, read_registration, verify_database_association,
};

const MAX_SSE_FRAME_BYTES: usize = 256 * 1024;
const MAX_SSE_CHUNK_BYTES: usize = 16 * 1024;
const MAX_BUFFERED_EVENTS: usize = 64;
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum EventStreamError {
    Registration(RegistrationError),
    Association(DatabaseAssociationError),
    Probe(ProbeError),
    Request,
    Redirect,
    HttpStatus(u16),
    NotEventStream,
    RegistrationChanged,
    Body,
    StreamEnded,
    Sse(SseError),
    CapacityExceeded,
    Cancelled,
}

impl fmt::Display for EventStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registration(_) => {
                formatter.write_str("OpenCode service registration is unavailable")
            }
            Self::Association(_) => {
                formatter.write_str("OpenCode service is not proven to use this database")
            }
            Self::Probe(_) => formatter.write_str("OpenCode service identity probe failed"),
            Self::Request => formatter.write_str("OpenCode event stream request failed"),
            Self::Redirect => formatter.write_str("OpenCode event stream returned a redirect"),
            Self::NotEventStream => {
                formatter.write_str("OpenCode service did not return an event stream")
            }
            Self::RegistrationChanged => {
                formatter.write_str("OpenCode service registration changed during connection")
            }
            Self::Body => formatter.write_str("OpenCode event stream ended with a read error"),
            Self::StreamEnded => formatter.write_str("OpenCode event stream ended"),
            Self::CapacityExceeded => {
                formatter.write_str("OpenCode event stream exceeded its buffer limit")
            }
            Self::Cancelled => formatter.write_str("OpenCode event stream was cancelled"),
            Self::HttpStatus(status) => {
                write!(formatter, "OpenCode event stream returned HTTP {status}")
            }
            Self::Sse(error) => {
                write!(
                    formatter,
                    "OpenCode event stream frame was invalid: {error:?}"
                )
            }
        }
    }
}

impl std::error::Error for EventStreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registration(error) => Some(error),
            Self::Association(error) => Some(error),
            Self::Probe(error) => Some(error),
            _ => None,
        }
    }
}

/// An authenticated event stream bound to the selected native database.
///
/// The server does not replay events. A new stream is a new live boundary, and
/// an ended stream is a continuity gap. Callers must not use
/// `server.connected` as a history watermark.
pub struct EventStream {
    response: Option<Response>,
    decoder: SseDecoder,
    buffered: VecDeque<SseEvent>,
    registration: ServiceRegistration,
    service_info: ServiceInfo,
}

impl EventStream {
    pub fn registration(&self) -> &ServiceRegistration {
        &self.registration
    }

    pub fn service_info(&self) -> &ServiceInfo {
        &self.service_info
    }

    /// Return the next complete event. Ended feeds and malformed frames fail
    /// instead of looking like a healthy empty stream.
    pub async fn next_event(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<SseEvent, EventStreamError> {
        loop {
            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                self.decoder.cancel();
                self.buffered.clear();
                self.response.take();
                return Err(EventStreamError::Cancelled);
            }
            if let Some(event) = self.buffered.pop_front() {
                return Ok(event);
            }

            let Some(response) = self.response.as_mut() else {
                return Err(EventStreamError::StreamEnded);
            };
            let result = tokio::select! {
                result = response.chunk() => result,
                () = tokio::time::sleep(CANCEL_POLL_INTERVAL) => continue,
            };
            let chunk = match result {
                Ok(Some(chunk)) => chunk,
                Ok(None) => {
                    self.response.take();
                    return Err(EventStreamError::StreamEnded);
                }
                Err(_) => {
                    self.response.take();
                    return Err(EventStreamError::Body);
                }
            };
            for part in chunk.chunks(MAX_SSE_CHUNK_BYTES) {
                for event in self.decoder.push(part) {
                    match event {
                        Ok(event) => {
                            if self.buffered.len() >= MAX_BUFFERED_EVENTS {
                                self.decoder.cancel();
                                self.buffered.clear();
                                self.response.take();
                                return Err(EventStreamError::CapacityExceeded);
                            }
                            self.buffered.push_back(event);
                        }
                        Err(error) => {
                            self.decoder.cancel();
                            self.buffered.clear();
                            self.response.take();
                            return Err(EventStreamError::Sse(error));
                        }
                    }
                }
            }
        }
    }
}

/// Connect to `/api/event` only after proving that the registered process has
/// the selected regular SQLite file open. Recheck the registration and file
/// binding after the stream response opens.
pub async fn open_verified_event_stream(
    registration_path: &Path,
    database_path: &Path,
    connect_timeout: Duration,
) -> Result<EventStream, EventStreamError> {
    let original = read_registration(registration_path).map_err(EventStreamError::Registration)?;
    let registration_for_proof = registration_path.to_path_buf();
    let database_for_proof = database_path.to_path_buf();
    let first_info =
        verify_binding(registration_for_proof, database_for_proof, connect_timeout).await?;
    let current = read_registration(registration_path).map_err(EventStreamError::Registration)?;
    if current != original
        || first_info.pid != current.pid
        || current
            .version
            .as_ref()
            .is_some_and(|version| version != &first_info.version)
    {
        return Err(EventStreamError::RegistrationChanged);
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = Client::builder()
        .connect_timeout(connect_timeout)
        .read_timeout(READ_IDLE_TIMEOUT)
        .redirect(Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| EventStreamError::Request)?;
    let url = current
        .endpoint
        .base_url
        .join("/api/event")
        .map_err(|_| EventStreamError::Request)?;
    let mut request = client.get(url);
    if let Some(password) = &current.endpoint.password {
        request = request.basic_auth("opencode", Some(&password.0));
    }
    let response = tokio::time::timeout(connect_timeout, request.send())
        .await
        .map_err(|_| EventStreamError::Request)?
        .map_err(|_| EventStreamError::Request)?;
    if response.status().is_redirection() {
        return Err(EventStreamError::Redirect);
    }
    if !response.status().is_success() {
        return Err(EventStreamError::HttpStatus(response.status().as_u16()));
    }
    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(';').next().is_some_and(|media_type| {
                media_type.trim().eq_ignore_ascii_case("text/event-stream")
            })
        });
    if !is_event_stream {
        return Err(EventStreamError::NotEventStream);
    }

    let after_info = verify_binding(
        registration_path.to_path_buf(),
        database_path.to_path_buf(),
        connect_timeout,
    )
    .await?;
    let after = read_registration(registration_path).map_err(EventStreamError::Registration)?;
    if after != current || after_info != first_info {
        return Err(EventStreamError::RegistrationChanged);
    }

    Ok(EventStream {
        response: Some(response),
        decoder: SseDecoder::new(MAX_SSE_FRAME_BYTES),
        buffered: VecDeque::new(),
        registration: after,
        service_info: after_info,
    })
}

async fn verify_binding(
    registration_path: PathBuf,
    database_path: PathBuf,
    timeout: Duration,
) -> Result<ServiceInfo, EventStreamError> {
    tokio::task::spawn_blocking(move || {
        verify_database_association(&registration_path, &database_path, timeout)
    })
    .await
    .map_err(|_| EventStreamError::Probe(ProbeError::Request))?
    .map_err(EventStreamError::Association)
}
