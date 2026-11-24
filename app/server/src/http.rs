//! Just enough HTTP/1.1 for the server and the devserver: one request per
//! connection, answered and closed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

use anyhow::{Context, Result};

/// Requests with a larger body are refused before it is read.
pub const MAX_BODY: usize = 8 << 20;
/// The request line and headers together may not be longer than this.
const MAX_HEAD: u64 = 64 << 10;

#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    /// The target as sent, query string included.
    pub path: String,
    /// Header names lowercased; values as sent.
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The path without its query string.
    pub fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or("/")
    }

    #[cfg(test)]
    pub fn with(method: &str, path: &str, headers: &[(&str, &str)]) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: Vec::new(),
        }
    }
}

/// Read one request. `Ok(None)` when its body is over [`MAX_BODY`].
pub fn read(stream: &TcpStream) -> Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?.take(MAX_HEAD + MAX_BODY as u64));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let mut request = Request {
        method: parts.next().unwrap_or("").to_owned(),
        path: parts.next().unwrap_or("/").to_owned(),
        ..Request::default()
    };
    let mut head = line.len() as u64;
    loop {
        let mut line = String::new();
        head += reader.read_line(&mut line)? as u64;
        if head > MAX_HEAD {
            anyhow::bail!("headers too long");
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            request
                .headers
                .push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length: usize = request
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Ok(None);
    }
    request.body = vec![0; length];
    reader.read_exact(&mut request.body)?;
    Ok(Some(request))
}

pub fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    respond_with(stream, status, kind, &[], body)
}

pub fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body).context("write body")
}

/// A file of the built UI at `route`; `index.html` for `/`.
pub fn serve_file(stream: &mut TcpStream, dist: &Path, route: &str) -> Result<()> {
    let rel = route.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains("..") || rel.contains('\\') {
        return respond(stream, "404 Not Found", "text/plain", b"not found");
    }
    match std::fs::read(dist.join(rel)) {
        Ok(bytes) => respond(stream, "200 OK", mime(rel), &bytes),
        Err(_) => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
