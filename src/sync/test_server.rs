//! A tiny HTTP server for backend tests, used by the native transport tests.
//!
//! Each connection is answered with the next queued response, and every
//! request is recorded so tests can assert on method, path and body.

#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A recorded request.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: String,
}

/// A one-shot mock server bound to an ephemeral loopback port.
pub struct MockServer {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    responses: Arc<Mutex<VecDeque<(u16, String)>>>,
}

impl MockServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let responses: Arc<Mutex<VecDeque<(u16, String)>>> = Arc::new(Mutex::new(VecDeque::new()));

        let requests_task = Arc::clone(&requests);
        let responses_task = Arc::clone(&responses);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let Some(raw) = read_request(&mut socket).await else {
                    continue;
                };
                let text = String::from_utf8_lossy(&raw).to_string();
                let mut sections = text.splitn(2, "\r\n\r\n");
                let head = sections.next().unwrap_or_default();
                let body = sections.next().unwrap_or_default().to_string();
                let mut request_line = head.lines().next().unwrap_or("").split_whitespace();
                let method = request_line.next().unwrap_or("").to_string();
                let path = request_line.next().unwrap_or("").to_string();
                requests_task
                    .lock()
                    .unwrap()
                    .push(Request { method, path, body });

                let (status, payload) = responses_task
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| (500, "{}".to_string()));
                let reply = format!(
                    "HTTP/1.1 {status} mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        MockServer {
            url: format!("http://{address}"),
            requests,
            responses,
        }
    }

    /// Queue the next response.
    pub fn reply(&self, status: u16, body: impl Into<String>) {
        self.responses
            .lock()
            .unwrap()
            .push_back((status, body.into()));
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// The most recent request.
    pub fn last(&self) -> Request {
        self.requests.lock().unwrap().last().cloned().unwrap()
    }
}

async fn read_request(socket: &mut TcpStream) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = headers_end(&buffer) {
            let length = content_length(&buffer[..end]);
            if buffer.len() >= end + 4 + length {
                break;
            }
        }
    }
    if buffer.is_empty() {
        None
    } else {
        Some(buffer)
    }
}

/// The index of the `\r\n\r\n` that ends the request headers.
fn headers_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// The `Content-Length` in a header block, or `0`.
fn content_length(headers: &[u8]) -> usize {
    let text = String::from_utf8_lossy(headers);
    for line in text.lines() {
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}
