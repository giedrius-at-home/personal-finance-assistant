//! Development only. Serves `app/ui/dist` and `POST /invoke/<command>` on
//! 127.0.0.1 so the UI can be exercised in a browser against the real local
//! database. Never shipped in the app bundle. The command table is the
//! server's (`finance_server::commands`); bank sync, email and updates are
//! refused here.

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use finance_core::service::Service;
use finance_server::commands::{self, ok};
use finance_server::http::{self, respond};
use serde_json::Value;

/// One AI runner for the server's lifetime, without secret store access.
static ASSISTANT: std::sync::LazyLock<finance_ai::Assistant> =
    std::sync::LazyLock::new(|| finance_ai::Assistant::new(false));

fn main() -> Result<()> {
    let dist = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "app/ui/dist".into()),
    );
    let port = std::env::args().nth(2).unwrap_or_else(|| "1430".into());
    // DEV_DB points at a copy of the data, e.g. the parity golden.
    let db = std::env::var_os("DEV_DB")
        .map(PathBuf::from)
        .unwrap_or(finance_storage::default_db_path()?);
    let store = Arc::new(Service::new(db));
    let dist = Arc::new(dist);
    let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;
    eprintln!("finance-devserver on http://127.0.0.1:{port}");
    for stream in listener.incoming() {
        let mut stream = stream?;
        let (store, dist, port) = (Arc::clone(&store), Arc::clone(&dist), port.clone());
        // One thread per connection, so an idle pre-connect cannot block others.
        std::thread::spawn(move || {
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            if let Err(e) = handle(&mut stream, &store, &dist, &port) {
                eprintln!("request failed: {e}");
            }
        });
    }
    Ok(())
}

fn handle(stream: &mut TcpStream, store: &Service, dist: &Path, port: &str) -> Result<()> {
    let Some(request) = http::read(stream)? else {
        return respond(stream, "403 Forbidden", "text/plain", b"forbidden");
    };
    // Only this machine's own pages: other sites (or DNS rebinding) must
    // not drive writes against the real ledger.
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host = request.header("host").unwrap_or("").to_ascii_lowercase();
    let trusted = local.contains(&host)
        && request.header("origin").is_none_or(|o| {
            o.to_ascii_lowercase()
                .strip_prefix("http://")
                .is_some_and(|o| local.iter().any(|l| l == o))
        });
    if !trusted {
        return respond(stream, "403 Forbidden", "text/plain", b"forbidden");
    }
    if request.method == "POST"
        && let Some(cmd) = request.route().strip_prefix("/invoke/")
    {
        let args: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        return match run(store, cmd, args) {
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
    http::serve_file(stream, dist, request.route())
}

fn run(service: &Service, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
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
        // The forecast model only exists in the app.
        "forecast_model_status" => serde_json::to_value(finance_types::ForecastModelStatus {
            download_bytes: 1_322_900_097,
            ..Default::default()
        })
        .map_err(|e| e.to_string()),
        // The secret store belongs to the app; here the Banks screen shows the
        // stored runs only and syncs are refused.
        "bank_sync_status" => ok(service.bank_sync_report(Default::default())),
        "sync_bank"
        | "renew_bank_consent"
        | "adopt_bank_consents"
        | "set_bank_credentials"
        | "forget_bank_credentials"
        | "bank_link_start"
        | "bank_link_finish"
        | "bank_link_confirm" => Err("Bank sync runs in the app only.".to_owned()),
        // The secret store and the mail server belong to the app: settings can
        // be edited here, but no password is stored and nothing is sent.
        "email_status" => ok(service.email_status(false)),
        "save_email_settings" => {
            let saved =
                commands::arg(args, "settings").and_then(|s| service.save_email_settings(s));
            ok(saved.and_then(|_| service.email_status(false)))
        }
        "set_smtp_password" | "forget_smtp_password" | "send_test_email" => {
            Err("Email runs in the app only.".to_owned())
        }
        _ => commands::run(service, &ASSISTANT, cmd, args)
            .unwrap_or_else(|| Err(format!("unknown command {cmd}"))),
    }
}
