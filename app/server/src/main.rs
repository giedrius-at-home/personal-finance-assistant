//! The Finance server: the app's commands and the built UI over HTTP on a
//! private network (Tailscale), plus what the app does in the background —
//! bank sync, the daily email, AI suggestions, forecasts — so they run with
//! the laptop closed and the phone can open the same screens in a browser.
//!
//! Configuration, all from the environment (see `app/server/README.md`):
//! - `FINANCE_LISTEN`: address and port, never `0.0.0.0`.
//! - `FINANCE_UI_DIST`: the built UI (`app/ui/dist`).
//! - `FINANCE_SECRETS_DIR`: the secret store folder; must be set. Holds
//!   `server-token` and, once moved, the bank and mail secrets.
//! - `FINANCE_ACCESS_TEAM`, `FINANCE_ACCESS_AUD`, `FINANCE_ACCESS_EMAIL`
//!   (optional, all three): trust that Cloudflare Access login (`access.rs`).
//! - `FINANCE_DB` (optional): the ledger; default `<data dir>/Finance/finance.sqlite3`.
//! - `FINANCE_MODEL_DIR` (optional): the TimesFM weights; default
//!   `<data dir>/Finance/models/timesfm-3.0`.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use finance_ai::Assistant;
use finance_bank::{Credentials, Syncer};
use finance_core::daily::SeriesForecaster;
use finance_core::forecast_model::ForecastModel;
use finance_core::service::Service;
use finance_core::spending_email::{Message, Sender, Smtp};
use finance_notify::push::{self, Http, PushAlerts, PushMessage, Pusher, Vapid};
use finance_notify::{EmailAlerts, Fanout, Mailer, Ollama};
use finance_secrets::{Item, Secret};
use finance_server::access::{self, Access};
use finance_server::auth;
use finance_server::commands::{self, arg, ok};
use finance_server::http::{self, Request, respond, respond_with};
use finance_timesfm::{FILES, NAME, TimesFm};
use finance_types::notify::{DaySummary, EmailSettings, PushStatus, PushSubscription};
use serde_json::Value;

/// Bank sync checks for due banks this often; each bank syncs once a day.
const BANK_EVERY: Duration = Duration::from_secs(15 * 60);
/// The daily email's slot is checked this often.
const EMAIL_EVERY: Duration = Duration::from_secs(15 * 60);
/// A suggestion pass every minute, or sooner after a sync wrote transactions.
const AI_EVERY: Duration = Duration::from_secs(60);
/// Let the server settle before the first background run.
const FIRST: Duration = Duration::from_secs(60);
/// A wrong login waits this long, to slow guessing (the token is 256 bits).
const WRONG_LOGIN: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct App {
    service: Service,
    syncer: Syncer,
    assistant: Assistant,
    token: Secret,
    /// The owner's Cloudflare Access login, when configured (the phone).
    access: Option<Access>,
    dist: PathBuf,
    model_dir: PathBuf,
    /// Web Push to the installed web app; `None` when the key could not be
    /// made or read (logged at start).
    pusher: Option<Arc<Pusher<Http>>>,
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} must be set"))
}

fn main() -> Result<()> {
    let address = auth::listen_address(&required("FINANCE_LISTEN")?).map_err(anyhow::Error::msg)?;
    let dist = PathBuf::from(required("FINANCE_UI_DIST")?);
    if !dist.join("index.html").is_file() {
        bail!("FINANCE_UI_DIST has no index.html: {}", dist.display());
    }
    // Without it the store would fall back to a macOS path.
    required("FINANCE_SECRETS_DIR")?;
    let token = finance_secrets::get(Item::ServerToken)?
        .map(|t| Secret::new(t.expose().trim().to_owned()))
        .filter(|t| auth::valid_token(t.expose()))
        .context("the secret store needs a server-token of 32 or more letters, digits, - or _ (openssl rand -hex 32)")?;
    let db = match std::env::var_os("FINANCE_DB") {
        Some(path) => PathBuf::from(path),
        None => finance_storage::default_db_path()?,
    };
    let model_dir = match std::env::var_os("FINANCE_MODEL_DIR") {
        Some(path) => PathBuf::from(path),
        None => finance_storage::data_dir()?
            .join("models")
            .join("timesfm-3.0"),
    };
    // Lives as long as the process; every thread borrows it.
    let app: &'static App = Box::leak(Box::new(App {
        service: Service::new(db),
        syncer: Syncer::default(),
        assistant: Assistant::default(),
        token,
        access: Access::from_env().map_err(anyhow::Error::msg)?,
        dist,
        model_dir,
        pusher: pusher(),
    }));
    install_model(app);
    register_alerts(app);
    schedule(app);

    let listener = TcpListener::bind(address)?;
    eprintln!("finance-server on http://{address}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        std::thread::spawn(move || {
            stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
            if let Err(e) = handle(&mut stream, app) {
                eprintln!("request failed: {e}");
            }
        });
    }
    Ok(())
}

const LOGIN_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Finance</title>
<style>body{font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0}form{display:grid;gap:12px;width:min(320px,90vw)}input,button{font:inherit;padding:10px;border-radius:8px;border:1px solid #999}</style>
</head><body><form method="post" action="/login">
<label for="token">Server token</label>
<input id="token" name="token" type="password" autocomplete="current-password" required>
<button type="submit">Open Finance</button>
</form></body></html>"#;

fn handle(stream: &mut TcpStream, app: &App) -> Result<()> {
    let Some(request) = http::read(stream)? else {
        return respond(stream, "413 Content Too Large", "text/plain", b"too large");
    };
    let route = request.route();
    // Through the Cloudflare tunnel the Access login decides, alone: a
    // request that carries its header gets in on that or not at all, and is
    // never offered the token page.
    let via_access = request.header(access::HEADER);
    let by_access = match (&app.access, via_access) {
        (Some(access), Some(token)) => access.allows(token),
        _ => false,
    };
    if via_access.is_some() && !by_access {
        return respond(stream, "403 Forbidden", "text/plain", b"forbidden");
    }
    if route == "/login" {
        return if by_access {
            respond_with(
                stream,
                "303 See Other",
                "text/plain",
                &[("Location", "/")],
                b"",
            )
        } else {
            login(stream, app, &request)
        };
    }
    if !by_access && !auth::authorized(&request, app.token.expose()) {
        return if route.starts_with("/invoke/") {
            respond(
                stream,
                "401 Unauthorized",
                "application/json",
                br#""Sign in again.""#,
            )
        } else {
            respond_with(
                stream,
                "303 See Other",
                "text/plain",
                &[("Location", "/login")],
                b"",
            )
        };
    }
    if request.method == "POST"
        && let Some(cmd) = route.strip_prefix("/invoke/")
    {
        if !auth::same_origin(&request) {
            return respond(stream, "403 Forbidden", "text/plain", b"forbidden");
        }
        let args: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        return match run(app, cmd, args) {
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
    http::serve_file(stream, &app.dist, route)
}

/// The login page, and its form: the right token gets the session cookie.
fn login(stream: &mut TcpStream, app: &App, request: &Request) -> Result<()> {
    if request.method != "POST" {
        return respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            LOGIN_PAGE.as_bytes(),
        );
    }
    // Why a login was refused goes to the log (never the token itself), so a
    // failure behind a proxy can be told apart from a mistyped token.
    let refused = if !auth::same_origin(request) {
        Some(format!(
            "origin {:?} is not host {:?}",
            request.header("origin").unwrap_or(""),
            request.header("host").unwrap_or("")
        ))
    } else {
        match auth::login_token(&request.body) {
            None => Some(format!(
                "no usable token in a {}-byte form (64 letters and digits expected)",
                request.body.len()
            )),
            Some(t) if !auth::token_matches(t, app.token.expose()) => Some(format!(
                "a well-formed {}-character token did not match",
                t.len()
            )),
            Some(_) => None,
        }
    };
    if let Some(reason) = refused {
        eprintln!("login refused: {reason}");
        std::thread::sleep(WRONG_LOGIN);
        return respond_with(
            stream,
            "303 See Other",
            "text/plain",
            &[("Location", "/login")],
            b"",
        );
    }
    // HttpOnly: page scripts never see it. SameSite=Strict: other sites'
    // pages cannot send it. Not `Secure`: the private network is plain
    // HTTP, encrypted by WireGuard underneath.
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000",
        auth::COOKIE,
        app.token.expose()
    );
    respond_with(
        stream,
        "303 See Other",
        "text/plain",
        &[("Location", "/"), ("Set-Cookie", &cookie)],
        b"",
    )
}

/// Sends with the secret store's SMTP password, read only when an email
/// goes out (as the app's `StoredMailer`).
#[derive(Debug)]
struct StoredMailer(Smtp);

impl Sender for StoredMailer {
    fn send(&self, message: &Message) -> Result<(), String> {
        let password = finance_secrets::get(Item::SmtpPassword)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Save the SMTP password first.".to_owned())?;
        Mailer::smtp(self.0.clone(), &password)?.send(message)
    }
}

fn password_set() -> bool {
    matches!(finance_secrets::get(Item::SmtpPassword), Ok(Some(_)))
}

/// The commands that need the server's own parts; the rest are shared.
fn run(app: &App, cmd: &str, args: Value) -> Result<Value, String> {
    let (service, syncer) = (&app.service, &app.syncer);
    match cmd {
        "bank_sync_status" => ok(syncer.report(service)),
        "sync_bank" => {
            let result = arg::<String>(args, "bank").and_then(|bank| syncer.sync(service, &bank));
            if result
                .as_ref()
                .is_ok_and(|r| r.created.is_some_and(|n| n > 0))
            {
                app.assistant.wake();
            }
            ok(result)
        }
        "renew_bank_consent" => arg(args, "consent").and_then(|c| ok(syncer.renew(service, &c))),
        "adopt_bank_consents" => {
            Err("Saved consents are taken over on the Mac, before the move.".to_owned())
        }
        "set_bank_credentials" => arg(args, "credentials")
            .and_then(|c| Credentials::store(&c).map_err(|e| e.to_string()))
            .map(|()| Value::Null),
        "forget_bank_credentials" => Credentials::forget()
            .map_err(|e| e.to_string())
            .map(|()| Value::Null),
        // The address to approve at; the server has no browser to open it.
        "bank_link_start" => arg(args, "start").and_then(|s| ok(syncer.link_start(&s))),
        "bank_link_finish" => arg::<String>(args.clone(), "bank").and_then(|bank| {
            let address = arg::<String>(args, "address")?;
            ok(syncer.link_finish(service, &bank, &address))
        }),
        "bank_link_confirm" => {
            arg(args, "confirm").and_then(|c| ok(syncer.link_confirm(service, &c)))
        }
        "email_status" => ok(service.email_status(password_set())),
        "save_email_settings" => {
            let saved = arg(args, "settings").and_then(|s| service.save_email_settings(s));
            ok(saved.and_then(|_| service.email_status(password_set())))
        }
        "set_smtp_password" => arg::<String>(args, "password").and_then(|p| {
            let password = Secret::new(p);
            let length = password.expose().chars().count();
            if length == 0 || length > 256 {
                return Err("The password must be 1 to 256 characters.".into());
            }
            finance_secrets::set(Item::SmtpPassword, &password).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }),
        "forget_smtp_password" => finance_secrets::delete(Item::SmtpPassword)
            .map_err(|e| e.to_string())
            .map(|()| Value::Null),
        "send_test_email" => ok(send_test_email(service)),
        "push_status" => ok(push_status(app)),
        "push_subscribe" => arg::<PushSubscription>(args, "subscription").and_then(|s| {
            app.pusher
                .as_ref()
                .ok_or("Push is not set up on this server.")?;
            push::check_subscription(&s)?;
            service.save_push_subscription(&s)?;
            ok(push_status(app))
        }),
        "push_unsubscribe" => arg::<String>(args, "endpoint").and_then(|e| {
            service.forget_push_subscription(&e)?;
            ok(push_status(app))
        }),
        "push_test" => ok(push_test(app)),
        "forecast_model_status" => {
            let available = finance_timesfm::is_complete(&app.model_dir);
            let run = service.forecast_model_status().unwrap_or_default();
            ok(Ok(finance_types::ForecastModelStatus {
                available,
                source: available.then(|| "server".to_owned()),
                download_bytes: FILES.iter().map(|f| f.bytes).sum(),
                downloading: None,
                running: run.running,
                last_run_seconds: run.last_seconds,
                error: run.error,
            }))
        }
        "download_forecast_model" => Err(format!(
            "Copy the model's files to {} on the server and restart it.",
            app.model_dir.display()
        )),
        "update_status" => ok(Ok(finance_types::UpdateStatus {
            current_version: env!("CARGO_PKG_VERSION").into(),
            ..Default::default()
        })),
        "check_for_updates" | "install_update" | "set_release_token" | "forget_release_token" => {
            Err("The server is updated by deploying a new build.".to_owned())
        }
        _ => commands::run(service, &app.assistant, cmd, args)
            .unwrap_or_else(|| Err(format!("unknown command {cmd}"))),
    }
}

/// The app's "Send test": today's email, marked as a test, sent now.
fn send_test_email(service: &Service) -> Result<String, String> {
    let settings = service.email_settings()?;
    let settings = finance_core::spending_email::validate(EmailSettings {
        enabled: true,
        ..settings
    })
    .map_err(|e| e.to_string())?;
    let ollama = Ollama::local()?;
    let message = service.spending_email_test(jiff::Timestamp::now(), &settings, &ollama)?;
    StoredMailer(Smtp::from_settings(&settings)).send(&message)?;
    Ok(format!("Test email sent to {}.", settings.recipient))
}

/// The server's VAPID key from the secret store, made on first start.
fn pusher() -> Option<Arc<Pusher<Http>>> {
    let vapid = match finance_secrets::get(Item::VapidKey) {
        Ok(Some(stored)) => Vapid::from_pkcs8(stored.expose()),
        Ok(None) => Vapid::generate().and_then(|(vapid, stored)| {
            finance_secrets::set(Item::VapidKey, &Secret::new(stored))
                .map_err(|e| e.to_string())?;
            Ok(vapid)
        }),
        Err(e) => Err(e.to_string()),
    };
    match vapid.and_then(|v| Ok(Pusher::new(v, Http::new()?))) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            eprintln!("web push: off ({e})");
            None
        }
    }
}

fn forget_gone(app: &App, s: &PushSubscription) {
    eprintln!(
        "web push: forgetting a device at {}",
        finance_core::push::device_label(&s.endpoint)
    );
    let _ = app.service.forget_push_subscription(&s.endpoint);
}

/// Alerts go to every phone that turned on notifications, and by email when
/// "Email budget and goal alerts" is on. Every evaluation (after a bank
/// sync, after a refresh) records how delivery went.
fn register_alerts(app: &'static App) {
    let mut channels: Vec<Box<dyn finance_core::alerts::Notifier + Send + Sync>> =
        vec![Box::new(EmailAlerts::new(
            move || app.service.email_settings(),
            |smtp: &Smtp, message: &Message| {
                let sent = StoredMailer(smtp.clone()).send(message);
                if let Err(e) = &sent {
                    eprintln!("alert email failed: {e}");
                }
                sent
            },
        ))];
    if let Some(pusher) = &app.pusher {
        channels.push(Box::new(PushAlerts::new(
            pusher.clone(),
            move || app.service.push_subscriptions(),
            move |s: &PushSubscription| forget_gone(app, s),
        )));
    }
    app.service.set_alert_notifier(Arc::new(Fanout(channels)));
}

/// The evening overspend summary: to every phone, and by email when "Email
/// budget and goal alerts" is on and today's risk email did not carry it.
fn deliver_summary(app: &App, summary: &DaySummary, now: jiff::Timestamp) -> bool {
    let (title, body) = finance_core::day_summary::push_text(summary);
    let mut sent = false;
    if let Some(pusher) = &app.pusher {
        let devices = app.service.push_subscriptions().unwrap_or_default();
        let message = PushMessage {
            title: title.clone(),
            body,
            url: "/".into(),
            tag: "day-summary".into(),
            urgent: false,
        };
        sent |= pusher.push_all(&devices, &message, now.as_second(), &|s| {
            forget_gone(app, s)
        }) == finance_core::alerts::Outcome::Delivered;
    }
    let emailed = app.service.email_settings().and_then(|settings| {
        if !settings.alerts || app.service.risk_email_sent_today(now)? {
            return Ok(false);
        }
        let settings =
            finance_core::spending_email::validate(settings).map_err(|e| e.to_string())?;
        let mut text = finance_core::day_summary::text_lines(summary).join("\n");
        text.push_str(&format!(
            "\n\nOpen your plan: {}\n",
            finance_core::spending_email::PLAN_LINK
        ));
        StoredMailer(Smtp::from_settings(&settings)).send(&Message {
            subject: title,
            body: text,
            html_body: None,
        })?;
        Ok(true)
    });
    match emailed {
        Ok(e) => sent |= e,
        Err(e) => eprintln!("day summary email failed: {e}"),
    }
    sent
}

fn push_status(app: &App) -> Result<PushStatus, String> {
    Ok(PushStatus {
        public_key: app.pusher.as_ref().map(|p| p.public_key()),
        devices: app
            .service
            .push_subscriptions()?
            .iter()
            .map(|s| finance_core::push::device_label(&s.endpoint))
            .collect(),
    })
}

/// The card's "Send a test": one notification to every device.
fn push_test(app: &App) -> Result<String, String> {
    let pusher = app
        .pusher
        .as_ref()
        .ok_or("Push is not set up on this server.")?;
    let devices = app.service.push_subscriptions()?;
    if devices.is_empty() {
        return Err("Turn on notifications on a phone first.".into());
    }
    let message = PushMessage {
        title: "Finance".into(),
        body: "Notifications work on this device.".into(),
        url: "/".into(),
        tag: "test".into(),
        urgent: true,
    };
    let outcome = pusher.push_all(
        &devices,
        &message,
        jiff::Timestamp::now().as_second(),
        &|s| forget_gone(app, s),
    );
    match outcome {
        finance_core::alerts::Outcome::Delivered => Ok("Test notification sent.".into()),
        _ => Err("The push service did not take the test notification.".into()),
    }
}

/// Give the service the forecast model when its weights are on the server.
fn install_model(app: &'static App) {
    if !finance_timesfm::is_complete(&app.model_dir) {
        eprintln!(
            "forecast model: none at {}; simple forecasts only",
            app.model_dir.display()
        );
        return;
    }
    let dir = app.model_dir.clone();
    app.service.set_forecast_model(Some(ForecastModel {
        name: NAME.into(),
        supports_covariates: true,
        load: Box::new(move || {
            TimesFm::load(&dir)
                .map(|m| Box::new(m) as Box<dyn SeriesForecaster + Send>)
                .map_err(|e| e.to_string())
        }),
        on_done: Box::new(move |ready| {
            let seconds = app
                .service
                .forecast_model_status()
                .and_then(|s| s.last_seconds)
                .unwrap_or_default();
            eprintln!(
                "forecast model run {} in {seconds:.1}s",
                if ready { "ok" } else { "failed" }
            );
        }),
    }));
}

/// What the app does while its window is open, for as long as the server runs.
fn schedule(app: &'static App) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST);
        loop {
            let results = app.syncer.sync_due(&app.service);
            for r in &results {
                eprintln!("scheduled sync {} {:?}", r.bank, r.status);
            }
            if results.iter().any(|r| r.created.is_some_and(|n| n > 0)) {
                app.assistant.wake();
            }
            if !results.is_empty() {
                // Due alerts go out by email when the owner turned that on.
                let _ = app.service.evaluate_alerts_now();
            }
            std::thread::sleep(BANK_EVERY);
        }
    });
    std::thread::spawn(move || {
        std::thread::sleep(FIRST);
        loop {
            let outcome = app.service.email_settings().and_then(|settings| {
                let ollama = Ollama::local()?;
                let mailer = StoredMailer(Smtp::from_settings(&settings));
                app.service
                    .run_spending_email(jiff::Timestamp::now(), &ollama, &mailer)
            });
            if let Err(e) = outcome {
                eprintln!("spending email failed: {e}");
            }
            let now = jiff::Timestamp::now();
            match app
                .service
                .run_day_summary(now, &|summary| deliver_summary(app, summary, now))
            {
                Ok(true) => eprintln!("day summary sent"),
                Ok(false) => {}
                Err(e) => eprintln!("day summary failed: {e}"),
            }
            std::thread::sleep(EMAIL_EVERY);
        }
    });
    std::thread::spawn(move || {
        app.assistant.wait(FIRST);
        loop {
            match app.assistant.tick(&app.service) {
                Ok(Some(run)) => eprintln!(
                    "ai pass suggested={} applied={} {}ms {}",
                    run.suggested,
                    run.applied,
                    run.millis,
                    if run.error.is_some() { "failed" } else { "ok" }
                ),
                Ok(None) => {}
                Err(_) => eprintln!("ai pass failed"),
            }
            app.assistant.wait(AI_EVERY);
        }
    });
}
