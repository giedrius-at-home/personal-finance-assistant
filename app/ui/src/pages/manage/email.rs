//! Manage → Settings → Daily email: the Rafael spending email the Python
//! service sent (its `PFA_SPENDING_EMAIL_*` and `PFA_SMTP_*` settings), with
//! the SMTP password kept in the Keychain and a "Send test" button.

use finance_types::notify::{EmailSettings, EmailStatus, SmtpSecurity};
use leptos::html::Input as InputElement;
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{FIELDSET, Messages, Ops, local_date};
use crate::api::{NoArgs, load};
use crate::ds::{
    Button, CONTROL, Card, CardTitle, Checkbox, Eyebrow, Field, Heading, Input, Select, Size,
    Variant, cn,
};

const GRID: &str = "my-5 grid gap-5 md:grid-cols-3";

/// The form as typed.
#[derive(Clone, Copy, Debug)]
struct Draft {
    enabled: RwSignal<bool>,
    recipient: RwSignal<String>,
    sender: RwSignal<String>,
    host: RwSignal<String>,
    port: RwSignal<String>,
    security: RwSignal<String>,
    username: RwSignal<String>,
    hour: RwSignal<String>,
    digest: RwSignal<bool>,
    ollama: RwSignal<bool>,
}

impl Draft {
    fn new() -> Self {
        let s = || RwSignal::new(String::new());
        Self {
            enabled: RwSignal::new(false),
            recipient: s(),
            sender: s(),
            host: s(),
            port: s(),
            security: s(),
            username: s(),
            hour: s(),
            digest: RwSignal::new(true),
            ollama: RwSignal::new(true),
        }
    }

    fn fill(self, s: &EmailSettings) {
        self.enabled.set(s.enabled);
        self.recipient.set(s.recipient.clone());
        self.sender.set(s.sender.clone());
        self.host.set(s.smtp_host.clone());
        self.port.set(s.smtp_port.to_string());
        self.security.set(
            match s.smtp_security {
                SmtpSecurity::Ssl => "ssl",
                SmtpSecurity::Starttls => "starttls",
            }
            .into(),
        );
        self.username.set(s.username.clone());
        self.hour.set(s.hour.to_string());
        self.digest.set(s.daily_digest);
        self.ollama.set(s.ollama);
    }

    fn settings(self) -> Result<EmailSettings, String> {
        let port = self
            .port
            .get_untracked()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or("The SMTP port must be between 1 and 65535.")?;
        let hour = self
            .hour
            .get_untracked()
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|h| *h <= 23)
            .ok_or("The email hour must be between 0 and 23.")?;
        Ok(EmailSettings {
            enabled: self.enabled.get_untracked(),
            recipient: self.recipient.get_untracked(),
            sender: self.sender.get_untracked(),
            smtp_host: self.host.get_untracked(),
            smtp_port: port,
            smtp_security: if self.security.get_untracked() == "starttls" {
                SmtpSecurity::Starttls
            } else {
                SmtpSecurity::Ssl
            },
            username: self.username.get_untracked(),
            hour,
            daily_digest: self.digest.get_untracked(),
            ollama: self.ollama.get_untracked(),
        })
    }
}

#[derive(Serialize)]
struct SettingsArgs {
    settings: EmailSettings,
}

#[derive(Serialize)]
struct PasswordArgs {
    password: String,
}

/// One checkbox line, as the preferences card has them.
fn check(
    label: &'static str,
    hint: &'static str,
    value: RwSignal<bool>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <label class="flex items-start gap-2.5 text-sm leading-relaxed">
            <span class="mt-1 flex">
                <Checkbox
                    checked=Signal::derive(move || value.get())
                    disabled=Signal::derive(move || busy.get())
                    on_change=Callback::new(move |v| value.set(v))
                />
            </span>
            <span>
                {label}
                <span class="block text-xs text-fg-muted">{hint}</span>
            </span>
        </label>
    }
}

#[component]
pub fn EmailCard() -> impl IntoView {
    let ops = Ops::new();
    let busy = ops.busy;
    let changed = RwSignal::new(0_u32);
    let status = load::<EmailStatus, _>("email_status", move || {
        changed.track();
        NoArgs {}
    });
    let draft = Draft::new();
    Effect::new(move |_| {
        if let Some(Ok(s)) = status.data.get() {
            draft.fill(&s.settings);
        }
    });
    let save = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        match draft.settings() {
            Ok(settings) => ops.run(
                "save_email_settings",
                SettingsArgs { settings },
                move |_: EmailStatus| {
                    ops.notice.set("Email settings saved.".into());
                    changed.update(|n| *n += 1);
                },
            ),
            Err(e) => ops.error.set(e),
        }
    };
    let password = NodeRef::<InputElement>::new();
    let save_password = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(el) = password.get() else { return };
        let value = el.value();
        el.set_value("");
        ops.run(
            "set_smtp_password",
            PasswordArgs { password: value },
            move |(): ()| {
                ops.notice.set("Password saved in your Keychain.".into());
                changed.update(|n| *n += 1);
            },
        );
    };
    let forget_password = move |_| {
        ops.run("forget_smtp_password", NoArgs {}, move |(): ()| {
            ops.notice
                .set("Password removed from your Keychain.".into());
            changed.update(|n| *n += 1);
        });
    };
    let send_test = move |_| {
        ops.run("send_test_email", NoArgs {}, move |message: String| {
            ops.notice.set(message);
        });
    };
    let securities = Signal::derive(|| {
        vec![
            ("ssl".to_owned(), "SSL/TLS (port 465)".to_owned()),
            ("starttls".to_owned(), "STARTTLS (port 587)".to_owned()),
        ]
    });
    let body = move |s: EmailStatus| {
        let password_set = s.password_set;
        let ready = password_set && !s.settings.recipient.is_empty();
        let facts = [
            ("Last spending email", local_date(s.last_alert.as_deref())),
            ("Last forecast digest", local_date(s.last_digest.as_deref())),
            ("Last daily check", local_date(s.last_run.as_deref())),
        ];
        view! {
            <form on:submit=save>
                <fieldset disabled=move || busy.get() class=FIELDSET>
                    <div class="mt-5">
                        {check(
                            "Send the daily spending email",
                            "Once a day after the hour below: a warning when spending runs ahead of the plan, otherwise the forecast digest.",
                            draft.enabled,
                            busy,
                        )}
                    </div>
                    <div class=GRID>
                        <Field label="Send to" id="mail-to">
                            <Input id="mail-to" value=draft.recipient kind="email" placeholder="you@example.com" />
                        </Field>
                        <Field label="Send from" id="mail-from" hint="Empty sends from the SMTP username.">
                            <Input id="mail-from" value=draft.sender kind="email" />
                        </Field>
                        <Field label="Email hour" id="mail-hour" hint="Local time; a missed day is caught up once.">
                            <Input id="mail-hour" value=draft.hour kind="number" min="0" max="23" />
                        </Field>
                        <Field label="SMTP host" id="mail-host">
                            <Input id="mail-host" value=draft.host placeholder="smtp.gmail.com" />
                        </Field>
                        <Field label="SMTP port" id="mail-port">
                            <Input id="mail-port" value=draft.port kind="number" min="1" max="65535" />
                        </Field>
                        <Field label="Security" id="mail-security">
                            <Select id="mail-security" value=draft.security options=securities />
                        </Field>
                        <Field label="SMTP username" id="mail-user">
                            <Input id="mail-user" value=draft.username />
                        </Field>
                    </div>
                    <div class="mb-5 grid gap-3">
                        {check(
                            "Forecast digest on quiet days",
                            "Without a warning, send the next days' forecast instead.",
                            draft.digest,
                            busy,
                        )}
                        {check(
                            "Let the local Qwen model write the opening lines",
                            "Ollama on this Mac writes two sentences without figures; every amount is added by the app. Off, or if the model is not running, fixed text is used.",
                            draft.ollama,
                            busy,
                        )}
                    </div>
                    <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || busy.get())>
                        "Save email settings"
                    </Button>
                </fieldset>
            </form>
            <div class="mt-5 grid gap-3 border-t border-line pt-5">
                <p class="text-sm text-fg-muted">
                    {if password_set {
                        "An SMTP password is saved in your Keychain. For Gmail, use an app password."
                    } else {
                        "No SMTP password saved. For Gmail, create an app password; it is kept in your Keychain."
                    }}
                </p>
                <form class="flex flex-col gap-3 sm:flex-row sm:items-end" on:submit=save_password>
                    <input
                        node_ref=password
                        class=cn([CONTROL, "sm:max-w-sm"])
                        type="password"
                        autocomplete="off"
                        placeholder="SMTP password"
                        aria-label="SMTP password"
                    />
                    <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || busy.get())>
                        {if password_set { "Replace password" } else { "Save password" }}
                    </Button>
                    {password_set
                        .then(|| {
                            view! {
                                <Button
                                    variant=Variant::Ghost
                                    disabled=Signal::derive(move || busy.get())
                                    on_click=Callback::new(forget_password)
                                >
                                    "Remove password"
                                </Button>
                            }
                        })}
                </form>
            </div>
            <div class="mt-5 grid gap-3 border-t border-line pt-5">
                <dl class="grid gap-4 sm:grid-cols-3">
                    {facts
                        .into_iter()
                        .map(|(term, value)| {
                            view! {
                                <div>
                                    <dt class="text-xs text-fg-muted">{term}</dt>
                                    <dd class="mt-1.5 text-sm tabular-nums">{value}</dd>
                                </div>
                            }
                        })
                        .collect_view()}
                </dl>
                <div class="flex flex-wrap items-center gap-3">
                    <Button
                        size=Size::Sm
                        disabled=Signal::derive(move || busy.get() || !ready)
                        on_click=Callback::new(send_test)
                    >
                        {move || if busy.get() { "Working…" } else { "Send test" }}
                    </Button>
                    <span class="text-xs text-fg-muted">
                        {if ready {
                            "Sends today's email now, marked as a test, with the saved settings."
                        } else {
                            "Save a recipient and a password first."
                        }}
                    </span>
                </div>
            </div>
        }
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"RAFAEL"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Daily spending email"</CardTitle>
            <div class="mt-4 grid gap-3">
                <Messages ops=ops />
            </div>
            {move || match status.data.get() {
                None => view! { <p class="mt-5 text-sm text-fg-muted">"Loading email settings…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p class="mt-5 text-sm text-fg-muted">{format!("Email settings are not available ({e}).")}</p> }
                        .into_any()
                }
                Some(Ok(s)) => body(s).into_any(),
            }}
        </Card>
    }
}
