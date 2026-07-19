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
    UploadEcho,
}

pub struct FixtureServer {
    address: SocketAddr,
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    reached_stall: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
    unexpected_requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug)]
pub struct UploadRequest {
    pub body_bytes: usize,
    pub content_type: Option<String>,
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
        let captured = uploads.clone();
        let reached_stall = Arc::new(Notify::new());
        let server_reached_stall = reached_stall.clone();
        let request_count = Arc::new(AtomicUsize::new(0));
        let server_request_count = request_count.clone();
        let unexpected_requests = Arc::new(AtomicUsize::new(0));
        let server_unexpected_requests = unexpected_requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let plan = plan.clone();
                let captured = captured.clone();
                let reached_stall = server_reached_stall.clone();
                let request_count = server_request_count.clone();
                let unexpected_requests = server_unexpected_requests.clone();
                tokio::spawn(async move {
                    let _ = serve(
                        socket,
                        plan,
                        captured,
                        reached_stall,
                        request_count,
                        unexpected_requests,
                    )
                    .await;
                });
            }
        });
        Self {
            address,
            uploads,
            reached_stall,
            request_count,
            unexpected_requests,
            task,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub async fn uploads(&self) -> Vec<UploadRequest> {
        self.uploads.lock().await.clone()
    }

    pub async fn wait_until_stalled(&self) {
        self.reached_stall.notified().await;
    }

    pub fn unexpected_requests(&self) -> usize {
        self.unexpected_requests.load(Ordering::Relaxed)
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut socket: TcpStream,
    plan: ResponsePlan,
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    reached_stall: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
    unexpected_requests: Arc<AtomicUsize>,
) -> io::Result<()> {
    let (headers, initial_body) = read_request(&mut socket).await?;
    request_count.fetch_add(1, Ordering::Relaxed);
    match plan {
        ResponsePlan::CloudflareCompatible => {
            serve_cloudflare_compatible(
                &mut socket,
                &headers,
                initial_body,
                uploads,
                unexpected_requests,
            )
            .await?;
        }
        ResponsePlan::DelayHeaders => {
            reached_stall.notify_one();
            std::future::pending::<()>().await;
        }
        ResponsePlan::StallBody => {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .await?;
            reached_stall.notify_one();
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
            reached_stall.notify_one();
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
            for _ in 0..chunks {
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
            uploads.lock().await.push(UploadRequest {
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
