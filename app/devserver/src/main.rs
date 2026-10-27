//! Development only. Serves `app/ui/dist` and `POST /invoke/<command>` on
//! 127.0.0.1 so the UI can be exercised in a browser against the real local
//! database. Never shipped in the app bundle.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use finance_core::service::Service;
use finance_types::report::review::TransactionSearchQuery;
use serde::Deserialize;
use serde_json::Value;

/// One AI runner for the server's lifetime, without Keychain access.
static ASSISTANT: std::sync::LazyLock<finance_ai::Assistant> =
    std::sync::LazyLock::new(|| finance_ai::Assistant::new(false));

#[derive(Deserialize)]
struct PlanArgs {
    plan: finance_types::gateway::BudgetPlan,
}

#[derive(Deserialize)]
struct PlannedPaymentArgs {
    request: finance_types::gateway::PlannedPaymentRequest,
}

#[derive(Deserialize)]
struct ArchivePlanArgs {
    plan_id: String,
}

#[derive(Deserialize)]
struct MonthArgs {
    month: String,
}

#[derive(Deserialize)]
struct ClosureActionArgs {
    action: finance_types::report::closure::ClosureAction,
}

#[derive(Deserialize)]
struct PotArgs {
    pot: finance_types::report::closure::PotDraft,
}

#[derive(Deserialize)]
struct RoleArgs {
    role: finance_types::report::closure::AccountRoleDraft,
}

#[derive(Deserialize)]
struct SearchArgs {
    query: TransactionSearchQuery,
}

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

fn handle(stream: &mut TcpStream, store: &Service, dist: &Path) -> Result<()> {
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
        let result = run(store, cmd, args);
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

fn ok<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

fn run(service: &Service, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
        "dashboard" => ok(service.dashboard()),
        "review" => ok(service.review()),
        "status" => ok(service.status()),
        "refresh" => {
            service.refresh();
            // No notifier here: due alerts are recorded as `unavailable`
            // (or `disabled`), never shown.
            let _ = service.evaluate_alerts_now();
            Ok(Value::Null)
        }
        "save_budget_plan" => {
            let a: PlanArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.save_budget_plan(a.plan))
        }
        "save_spending_group" => {
            let a: GroupArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.save_spending_group(&a.category, a.group.as_deref(), a.revision))
        }
        "create_spending_group" => {
            let a: NewGroupArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.create_spending_group(&a.group))
        }
        "create_planned_payment" => {
            let a: PlannedPaymentArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.create_planned_payment(&a.request))
        }
        "archive_planned_payment" => {
            let a: ArchivePlanArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.archive_planned_payment(&a.plan_id))
        }
        "search_transactions" => {
            let a: SearchArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.search(&a.query))
        }
        "closure" => {
            let a: MonthArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.closure(&a.month))
        }
        "closure_action" => {
            let a: ClosureActionArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.closure_action(a.action))
        }
        "closure_pots" => ok(service.pots()),
        "save_pot" => {
            let a: PotArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.save_pot(a.pot))
        }
        "save_account_role" => {
            let a: RoleArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
            ok(service.save_account_role(a.role))
        }
        "unclassified_accounts" => ok(service.unclassified_accounts()),
        "personal_transactions" => {
            ok(service.personal_transactions(&arg::<String>(args.clone(), "query")?))
        }
        "save_personal_choice" => ok(service.save_personal_choice(arg(args.clone(), "choice")?)),
        "set_transaction_category" => {
            ok(service.set_transaction_category(arg(args.clone(), "change")?))
        }
        "review_decisions" => ok(service.review_decisions(arg(args.clone(), "decisions")?)),
        "review_batch" => ok(service.review_batch(arg(args.clone(), "items")?)),
        "review_category_options" => {
            ok(service.review_category_options(&arg::<String>(args.clone(), "proposal")?))
        }
        "save_review_details" => ok(service.save_review_details(arg(args.clone(), "details")?)),
        "audit_daily_review" => {
            ok(service.audit_daily_review(&arg::<String>(args.clone(), "transaction")?))
        }
        "preview_consolidation" => {
            ok(service.preview_consolidation(&arg(args.clone(), "request")?))
        }
        "apply_consolidation" => ok(service.apply_consolidation(arg(args.clone(), "request")?)),
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
        "create_bill" => ok(service.create_bill(request(args)?)),
        "edit_bill" => ok(service.edit_bill(request(args)?)),
        "archive_bill" => ok(service.archive_bill(request(args)?)),
        "save_subscription_categories" => ok(service.save_subscription_categories(request(args)?)),
        "link_bill_payment" => ok(service.link_bill_payment(request(args)?)),
        "decline_bill_match" => ok(service.decline_bill_match(request(args)?)),
        "manage_workspace" => ok(service.manage_workspace()),
        "banks" => ok(service.banks()),
        // The Keychain belongs to the app; here the Banks screen shows the
        // stored runs only and syncs are refused.
        "bank_sync_status" => ok(service.bank_sync_report(Default::default())),
        "reconcile_bank_account" => ok(service.reconcile_bank_account(
            &arg::<String>(args.clone(), "bank")?,
            arg::<i64>(args, "account_id")?,
        )),
        "sync_bank"
        | "renew_bank_consent"
        | "adopt_bank_consents"
        | "set_bank_credentials"
        | "forget_bank_credentials"
        | "bank_link_start"
        | "bank_link_finish"
        | "bank_link_confirm" => Err("Bank sync runs in the app only.".to_owned()),
        // AI suggestions against the local Ollama; TypeSafe needs the
        // Keychain, so its key cannot be set here.
        "ai_status" => ok(ASSISTANT.status(service)),
        "save_ai_settings" => ok(ASSISTANT.save_settings(service, arg(args, "settings")?)),
        "set_typesafe_key" => ok(ASSISTANT.set_typesafe_key(&arg::<String>(args, "key")?)),
        "forget_typesafe_key" => ok(ASSISTANT.forget_typesafe_key()),
        "suggest_categories" => ok(ASSISTANT.suggest_now(service)),
        "suggest_spending_group" => {
            ok(ASSISTANT.suggest_group(service, &arg::<String>(args, "category")?))
        }
        "research_merchant" => ok(ASSISTANT.research_merchant(
            service,
            &arg::<String>(args.clone(), "proposal")?,
            &arg::<String>(args, "public_name")?,
        )),
        "preview_csv_import" => ok(service.preview_csv_import(arg(args, "upload")?)),
        "commit_csv_import" => ok(service.commit_csv_import(arg(args, "commit")?)),
        "preview_settings" => ok(service.preview_settings(arg(args, "form")?)),
        "apply_settings" => ok(service.apply_settings(arg(args, "apply")?)),
        "rollback_settings" => ok(service.rollback_settings(arg(args, "revision")?)),
        // The Keychain and the mail server belong to the app: settings can
        // be edited here, but no password is stored and nothing is sent.
        "email_status" => ok(service.email_status(false)),
        "save_email_settings" => {
            let saved = service.save_email_settings(arg(args, "settings")?);
            ok(saved.and_then(|_| service.email_status(false)))
        }
        "set_smtp_password" | "forget_smtp_password" | "send_test_email" => {
            Err("Email runs in the app only.".to_owned())
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

/// The command argument `name`, as Tauri would pass it.
fn arg<T: serde::de::DeserializeOwned>(mut args: Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get_mut(name).map(Value::take).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
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

#[derive(Deserialize)]
struct GroupArgs {
    category: String,
    group: Option<String>,
    revision: u32,
}

#[derive(Deserialize)]
struct NewGroupArgs {
    group: String,
}

/// The `request` argument of a write command.
fn request<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    #[derive(Deserialize)]
    struct Args<T> {
        request: T,
    }
    serde_json::from_value::<Args<T>>(args)
        .map(|a| a.request)
        .map_err(|e| e.to_string())
}
