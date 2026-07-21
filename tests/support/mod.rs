#![allow(dead_code)]

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::sync::Notify;

#[derive(Clone)]
pub enum ResponsePlan {
    CloudflareCompatible,
    Exact {
        status: u16,
        body_bytes: usize,
        chunk_bytes: usize,
        server_timing: Option<&'static str>,
    },
    DeclaredLength {
        declared_bytes: usize,
        body_bytes: usize,
    },
    DelayHeaders,
    StallBody,
    StallUploadResponse,
    Trickle {
        chunks: usize,
        chunk_interval: Duration,
    },
    MultiServerTiming,
    EarlyUploadSuccess,
    UploadEcho,
    Metadata {
        status: u16,
        body: Vec<u8>,
        chunk_bytes: usize,
        chunk_delay: Duration,
    },
}

pub struct FixtureServer {
    address: SocketAddr,
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    reached_stall: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
    unexpected_requests: Arc<AtomicUsize>,
    response_chunk_count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct FixtureState {
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    reached_stall: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
    unexpected_requests: Arc<AtomicUsize>,
    response_chunk_count: Arc<AtomicUsize>,
}

#[derive(Clone, Debug)]
pub struct UploadRequest {
    pub body_bytes: usize,
    pub content_type: Option<String>,
    pub accept_encoding: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub referer: Option<String>,
    pub origin: Option<String>,
    pub authorization: Option<String>,
    pub accept_encoding: Option<String>,
}

impl FixtureServer {
    pub async fn cloudflare_compatible() -> Self {
        Self::start(ResponsePlan::CloudflareCompatible).await
    }

    pub async fn start(plan: ResponsePlan) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let uploads = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reached_stall = Arc::new(Notify::new());
        let request_count = Arc::new(AtomicUsize::new(0));
        let unexpected_requests = Arc::new(AtomicUsize::new(0));
        let response_chunk_count = Arc::new(AtomicUsize::new(0));
        let state = FixtureState {
            uploads: uploads.clone(),
            requests: requests.clone(),
            reached_stall: reached_stall.clone(),
            request_count: request_count.clone(),
            unexpected_requests: unexpected_requests.clone(),
            response_chunk_count: response_chunk_count.clone(),
        };
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let plan = plan.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, plan, state).await;
                });
            }
        });
        Self {
            address,
            uploads,
            requests,
            reached_stall,
            request_count,
            unexpected_requests,
            response_chunk_count,
            task,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn url_with_test_context(&self) -> String {
        format!("http://user:secret@{}/?query=secret#fragment", self.address)
    }

    pub async fn uploads(&self) -> Vec<UploadRequest> {
        self.uploads.lock().await.clone()
    }

    pub async fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().await.clone()
    }

    pub async fn wait_until_stalled(&self) {
        self.reached_stall.notified().await;
    }

    pub async fn wait_until_first_body_chunk(&self) {
        self.reached_stall.notified().await;
    }

    pub fn unexpected_requests(&self) -> usize {
        self.unexpected_requests.load(Ordering::Relaxed)
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }

    pub fn response_chunk_count(&self) -> usize {
        self.response_chunk_count.load(Ordering::Relaxed)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(mut socket: TcpStream, plan: ResponsePlan, state: FixtureState) -> io::Result<()> {
    let (headers, initial_body) = read_request(&mut socket).await?;
    state.request_count.fetch_add(1, Ordering::Relaxed);
    let request_line = headers.lines().next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    state.requests.lock().await.push(CapturedRequest {
        method: request_parts.next().unwrap_or_default().to_owned(),
        path: request_parts.next().unwrap_or_default().to_owned(),
        referer: header(&headers, "referer").map(ToOwned::to_owned),
        origin: header(&headers, "origin").map(ToOwned::to_owned),
        authorization: header(&headers, "authorization").map(ToOwned::to_owned),
        accept_encoding: header(&headers, "accept-encoding").map(ToOwned::to_owned),
    });
    match plan {
        ResponsePlan::CloudflareCompatible => {
            serve_cloudflare_compatible(
                &mut socket,
                &headers,
                initial_body,
                state.uploads.clone(),
                state.unexpected_requests.clone(),
            )
            .await?;
        }
        ResponsePlan::DelayHeaders => {
            state.reached_stall.notify_one();
            std::future::pending::<()>().await;
        }
        ResponsePlan::StallBody => {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .await?;
            state.reached_stall.notify_one();
            std::future::pending::<()>().await;
        }
        ResponsePlan::StallUploadResponse => {
            let content_length = header(&headers, "content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            let mut body_bytes = initial_body.len();
            let mut buffer = [0_u8; 8192];
            while body_bytes < content_length {
                let read = socket.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                body_bytes += read;
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .await?;
            state.reached_stall.notify_one();
            std::future::pending::<()>().await;
        }
        ResponsePlan::Trickle {
            chunks,
            chunk_interval,
        } => {
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {chunks}\r\n\r\n").as_bytes(),
                )
                .await?;
            if chunks > 0 {
                socket.write_all(&[0]).await?;
                state.reached_stall.notify_one();
            }
            for _ in 1..chunks {
                tokio::time::sleep(chunk_interval).await;
                socket.write_all(&[0]).await?;
            }
        }
        ResponsePlan::MultiServerTiming => {
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nServer-Timing: cache;desc=hit\r\nServer-Timing: cfRequestDuration;dur=2.5\r\n\r\n",
                )
                .await?;
        }
        ResponsePlan::EarlyUploadSuccess => {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        ResponsePlan::UploadEcho => {
            let content_length = header(&headers, "content-length")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            let mut body_bytes = initial_body.len();
            let mut buffer = [0_u8; 8192];
            while body_bytes < content_length {
                let read = socket.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                body_bytes += read;
            }
            state.uploads.lock().await.push(UploadRequest {
                body_bytes,
                content_type: header(&headers, "content-type").map(ToOwned::to_owned),
                accept_encoding: header(&headers, "accept-encoding").map(ToOwned::to_owned),
            });
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .await?;
        }
        ResponsePlan::DeclaredLength {
            declared_bytes,
            body_bytes,
        } => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {declared_bytes}\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(response.as_bytes()).await?;
            socket.write_all(&vec![0_u8; body_bytes]).await?;
        }
        ResponsePlan::Exact {
            status,
            body_bytes,
            chunk_bytes,
            server_timing,
        } => {
            let reason = if status == 200 { "OK" } else { "Error" };
            let timing = server_timing
                .map(|value| format!("Server-Timing: {value}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {body_bytes}\r\n{timing}\r\n"
            );
            socket.write_all(response.as_bytes()).await?;
            let chunk = vec![0_u8; chunk_bytes.max(1)];
            let mut remaining = body_bytes;
            while remaining > 0 {
                let emitted = remaining.min(chunk.len());
                socket.write_all(&chunk[..emitted]).await?;
                remaining -= emitted;
            }
        }
        ResponsePlan::Metadata {
            status,
            body,
            chunk_bytes,
            chunk_delay,
        } => {
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(response.as_bytes()).await?;
            for chunk in body.chunks(chunk_bytes.max(1)) {
                if !chunk_delay.is_zero() {
                    tokio::time::sleep(chunk_delay).await;
                }
                socket.write_all(chunk).await?;
                state.response_chunk_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    Ok(())
}

async fn serve_cloudflare_compatible(
    socket: &mut TcpStream,
    headers: &str,
    initial_body: Vec<u8>,
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    unexpected_requests: Arc<AtomicUsize>,
) -> io::Result<()> {
    let request_line = headers.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method == "GET" && target.starts_with("/__down?bytes=") {
        let bytes = target
            .split('?')
            .nth(1)
            .and_then(|query| {
                query.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "bytes").then_some(value)
                })
            })
            .and_then(|value| value.parse::<usize>().ok());
        let Some(bytes) = bytes else {
            unexpected_requests.fetch_add(1, Ordering::Relaxed);
            return write_empty_response(socket, 400).await;
        };
        tokio::time::sleep(Duration::from_millis(15)).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {bytes}\r\nServer-Timing: cfRequestDuration;dur=0\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(response.as_bytes()).await?;
        let chunk = [0_u8; 512];
        let mut remaining = bytes;
        while remaining > 0 {
            let emitted = remaining.min(chunk.len());
            socket.write_all(&chunk[..emitted]).await?;
            remaining -= emitted;
        }
        return Ok(());
    }

    if method == "POST" && target == "/__up" {
        let content_length = header(headers, "content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let mut body_bytes = initial_body.len();
        let mut buffer = [0_u8; 8192];
        while body_bytes < content_length {
            let read = socket.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            body_bytes += read;
        }
        uploads.lock().await.push(UploadRequest {
            body_bytes,
            content_type: header(headers, "content-type").map(ToOwned::to_owned),
            accept_encoding: header(headers, "accept-encoding").map(ToOwned::to_owned),
        });
        tokio::time::sleep(Duration::from_millis(15)).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nServer-Timing: cfRequestDuration;dur=0\r\nConnection: close\r\n\r\nOK",
            )
            .await?;
        return Ok(());
    }

    if method == "GET" && target == "/meta" {
        let body = br#"{"clientIp":"192.0.2.1","asn":64500,"asOrganization":"Fixture Network","country":"ZZ","city":"Test City","colo":{"iata":"TEST","cca2":"ZZ","city":"Test Edge"}}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await?;
        socket.write_all(body).await?;
        return Ok(());
    }

    unexpected_requests.fetch_add(1, Ordering::Relaxed);
    write_empty_response(socket, 404).await
}

async fn write_empty_response(socket: &mut TcpStream, status: u16) -> io::Result<()> {
    let reason = if status == 400 {
        "Bad Request"
    } else {
        "Not Found"
    };
    socket
        .write_all(
            format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
}

async fn read_request(socket: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let body = bytes.split_off(index + 4);
            bytes.truncate(index + 4);
            return Ok((String::from_utf8_lossy(&bytes).into_owned(), body));
        }
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), Vec::new()))
}

fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}
