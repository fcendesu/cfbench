use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

#[derive(Clone)]
pub enum ResponsePlan {
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
    UploadEcho,
}

pub struct FixtureServer {
    address: SocketAddr,
    uploads: Arc<Mutex<Vec<UploadRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug)]
pub struct UploadRequest {
    pub body_bytes: usize,
    pub content_type: Option<String>,
    pub accept_encoding: Option<String>,
}

impl FixtureServer {
    pub async fn start(plan: ResponsePlan) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let uploads = Arc::new(Mutex::new(Vec::new()));
        let captured = uploads.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let plan = plan.clone();
                let captured = captured.clone();
                tokio::spawn(async move {
                    let _ = serve(socket, plan, captured).await;
                });
            }
        });
        Self {
            address,
            uploads,
            task,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub async fn uploads(&self) -> Vec<UploadRequest> {
        self.uploads.lock().await.clone()
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
) -> io::Result<()> {
    let (headers, initial_body) = read_request(&mut socket).await?;
    match plan {
        ResponsePlan::DelayHeaders => {
            std::future::pending::<()>().await;
        }
        ResponsePlan::StallBody => {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .await?;
            std::future::pending::<()>().await;
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
