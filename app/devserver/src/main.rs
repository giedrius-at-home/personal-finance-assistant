//! Development only. Serves `app/ui/dist` and `POST /invoke/<command>` on
//! 127.0.0.1 so the UI can be exercised in a browser against the real local
//! database. Never shipped in the app bundle.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use finance_storage::Store;
use finance_types::TxQuery;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct TxArgs {
    query: TxQuery,
}

#[derive(Deserialize)]
struct MonthArgs {
    month: String,
}

fn main() -> Result<()> {
    let dist = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "app/ui/dist".into()),
    );
    let port = std::env::args().nth(2).unwrap_or_else(|| "1430".into());
    let store = Arc::new(Mutex::new(Store::open_read_only(
        &finance_storage::default_db_path()?,
    )?));
    let dist = Arc::new(dist);
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    eprintln!("finance-devserver on http://127.0.0.1:{port}");
    for stream in listener.incoming() {
        let mut stream = stream?;
        let (store, dist) = (Arc::clone(&store), Arc::clone(&dist));
        // One thread per connection, so an idle pre-connect cannot block others.
        std::thread::spawn(move || {
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            if let Err(e) = handle(&mut stream, &store, &dist) {
                eprintln!("request failed: {e}");
            }
        });
    }
    Ok(())
}

fn handle(stream: &mut TcpStream, store: &Mutex<Store>, dist: &Path) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));

    if method == "POST"
        && let Some(cmd) = path.strip_prefix("/invoke/")
    {
        let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let result = match store.lock() {
            Ok(store) => run(&store, cmd, args),
            Err(_) => Err("store-poisoned".to_owned()),
        };
        return match result {
            Ok(v) => respond(
                stream,
                "200 OK",
                "application/json",
                v.to_string().as_bytes(),
            ),
            Err(e) => respond(
                stream,
                "400 Bad Request",
                "application/json",
                Value::String(e).to_string().as_bytes(),
            ),
        };
    }
    let rel = path
        .split('?')
        .next()
        .unwrap_or("/")
        .trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains("..") {
        return respond(stream, "404 Not Found", "text/plain", b"not found");
    }
    match std::fs::read(dist.join(rel)) {
        Ok(bytes) => respond(stream, "200 OK", mime(rel), &bytes),
        Err(_) => respond(stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn ok<T: serde::Serialize>(result: finance_storage::Result<T>) -> Result<Value, String> {
    result
        .map_err(|e| e.to_string())
        .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

fn run(store: &Store, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
        "overview" => ok(store.overview()),
        "accounts" => ok(store.accounts()),
        "facets" => ok(store.facets()),
        "status" => ok(store.status()),
        "transactions" => {
            let a: TxArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(store.transactions(&a.query))
        }
        "category_totals" => {
            let a: MonthArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(store.category_totals(&a.month))
        }
        // Updates only exist in the app; show a plausible state for layout checks.
        "update_status" => {
            let available = std::env::var("DEV_FAKE_UPDATE").ok().map(|version| {
                finance_types::AvailableUpdate {
                    version,
                    notes: Some("Faster search and a calmer chart.".into()),
                    date: Some("2026-10-26".into()),
                }
            });
            serde_json::to_value(finance_types::UpdateStatus {
                current_version: "0.1.0".into(),
                token_set: available.is_some(),
                last_checked: Some("2026-10-26 21:00".into()),
                available,
                ..Default::default()
            })
            .map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    }
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body).context("write body")
}
