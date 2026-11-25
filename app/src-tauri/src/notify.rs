//! Alerts as macOS notifications, and the daily spending email: its
//! settings commands, the "Send test" button and the scheduler thread.
//! SMTP and Mistral block, so they run on their own threads, never on the
//! async runtime.

use std::sync::Arc;
use std::time::Duration;

use finance_ai::Assistant;
use finance_core::alerts::{Notification, Notifier, Outcome};
use finance_core::service::Service;
use finance_core::spending_email::{Message, Sender, Smtp};
use finance_notify::{Mailer, MistralWriter};
use finance_secrets::{Item, Secret};
use finance_types::notify::{EmailSettings, EmailStatus};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_notification::NotificationExt;

/// How often the scheduler looks at the daily slot (Python's loop).
const EVERY: Duration = Duration::from_secs(15 * 60);
/// Let the window load before the first check.
const FIRST: Duration = Duration::from_secs(90);

/// Native notifications through the notification plugin.
struct Desktop(AppHandle);

impl Notifier for Desktop {
    fn send(&self, message: &Notification) -> Outcome {
        // In Rafael's voice, as on the phone and in the email.
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date();
        let (title, body) =
            finance_core::rafael::alert(message, finance_core::rafael::seed(&message.name, today));
        match self
            .0
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            Ok(()) => Outcome::Delivered,
            Err(_) => Outcome::Failed,
        }
    }
}

/// Register desktop delivery with the service, so every alert evaluation
/// (after a bank sync, after a refresh) can notify.
pub fn register(app: &AppHandle) {
    app.state::<Service>()
        .set_alert_notifier(Arc::new(Desktop(app.clone())));
}

/// Evaluate the alerts on a plain thread.
pub fn evaluate_alerts(app: AppHandle) {
    std::thread::spawn(move || {
        let result = app.state::<Service>().evaluate_alerts_now();
        if crate::tracing() {
            eprintln!(
                "finance: alerts {}",
                if result.is_ok() {
                    "evaluated"
                } else {
                    "failed"
                }
            );
        }
    });
}

/// Sends with the secret store password, read only when an email goes out.
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

#[tauri::command(async)]
pub fn email_status(service: State<'_, Service>) -> Result<EmailStatus, String> {
    crate::traced("email_status", service.email_status(password_set()))
}

#[tauri::command(async)]
pub fn save_email_settings(
    service: State<'_, Service>,
    settings: EmailSettings,
) -> Result<EmailStatus, String> {
    let saved = service.save_email_settings(settings);
    crate::traced(
        "save_email_settings",
        saved.and_then(|_| service.email_status(password_set())),
    )
}

/// The SMTP password goes straight to the secret store and is never returned.
#[tauri::command(async)]
pub fn set_smtp_password(password: String) -> Result<(), String> {
    let password = Secret::new(password);
    let length = password.expose().chars().count();
    if length == 0 || length > 256 {
        return Err("The password must be 1 to 256 characters.".into());
    }
    finance_secrets::set(Item::SmtpPassword, &password).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn forget_smtp_password() -> Result<(), String> {
    finance_secrets::delete(Item::SmtpPassword).map_err(|e| e.to_string())
}

/// The owner's "Send test" button: today's email, marked as a test, sent
/// now with the saved settings. It changes no schedule or receipt.
#[tauri::command]
pub async fn send_test_email(app: AppHandle) -> Result<String, String> {
    let worker = std::thread::spawn(move || -> Result<String, String> {
        let service = app.state::<Service>();
        let settings = service.email_settings()?;
        let settings = finance_core::spending_email::validate(EmailSettings {
            enabled: true,
            ..settings
        })
        .map_err(|e| e.to_string())?;
        let message =
            service.spending_email_test(jiff::Timestamp::now(), &settings, &writer(&app))?;
        StoredMailer(Smtp::from_settings(&settings)).send(&message)?;
        Ok(format!("Test email sent to {}.", settings.recipient))
    });
    crate::traced(
        "send_test_email",
        tauri::async_runtime::spawn_blocking(move || worker.join())
            .await
            .map_err(|_| "Sending stopped".to_owned())?
            .map_err(|_| "Sending stopped".to_owned())?,
    )
}

/// The persona's email lines on Mistral; without a key the fixed lines stand in.
fn writer(app: &AppHandle) -> MistralWriter {
    MistralWriter {
        client: app
            .state::<Assistant>()
            .mistral(Duration::from_secs(60))
            .ok(),
        model: app
            .state::<Service>()
            .ai_settings()
            .map(|s| s.mistral_model)
            .unwrap_or_else(|_| finance_types::ai::DEFAULT_MISTRAL_MODEL.into()),
        name: app.state::<Service>().persona_name().into(),
    }
}

/// Check the daily slot every 15 minutes for as long as the app runs. The
/// service sends only when the owner turned the email on.
pub fn schedule(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST);
        loop {
            let service = app.state::<Service>();
            let outcome = service.email_settings().and_then(|settings| {
                let mailer = StoredMailer(Smtp::from_settings(&settings));
                service.run_spending_email(jiff::Timestamp::now(), &writer(&app), &mailer)
            });
            if crate::tracing() {
                match &outcome {
                    Ok(o) => eprintln!("finance: spending email {o:?}"),
                    Err(_) => eprintln!("finance: spending email failed"),
                }
            }
            std::thread::sleep(EVERY);
        }
    });
}
