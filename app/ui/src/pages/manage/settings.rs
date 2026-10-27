//! Manage → Settings: owner preferences with preview, apply, history and
//! rollback (personal-site `owner-operations.tsx`, "Settings"), plus the app
//! update panel and the local data facts.

use finance_types::UpdateStatus;
use finance_types::gateway::OwnerSettings;
use finance_types::manage::{ManageWorkspace, SettingsApply, SettingsForm, SettingsPreview};
use leptos::html::Input as InputElement;
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{DETAILS, FIELDSET, Ops, PREVIEW, SUMMARY, frame, load_workspace, local_date};
use crate::api::{NoArgs, load};
use crate::ds::{
    Button, CONTROL, Card, CardTitle, Checkbox, Eyebrow, Field, Heading, Input, Select, Size,
    TABLE, TABLE_CONTAINER, TCELL, TH, TROW, Variant, cn,
};
use crate::{format, state};

const GRID: &str = "my-5 grid gap-5 md:grid-cols-3";

fn label(key: &str) -> &str {
    match key {
        "timezone" => "Timezone",
        "payday_day" => "Monthly payday",
        "monthly_salary_eur" => "Monthly salary (EUR)",
        "salary_account_id" => "Salary account",
        "safety_buffer_eur" => "Safety buffer (EUR)",
        "evaluation_hour" => "Evaluation hour",
        "notifications_enabled" => "Desktop notifications",
        "heads_up_spent_ratio" => "Heads-up spent ratio",
        "heads_up_pace_ratio" => "Heads-up pace ratio",
        "heads_up_cooldown_hours" => "Heads-up cooldown (hours)",
        "warning_cooldown_hours" => "Warning cooldown (hours)",
        "critical_cooldown_hours" => "Critical cooldown (hours)",
        "forecast_excluded_categories" => "Not part of everyday spending",
        "policy_version" => "Policy version",
        other => other,
    }
}

/// Every setting as the site printed it: `String(value ?? "Not configured")`.
fn values(s: &OwnerSettings) -> Vec<(&'static str, String)> {
    let or = |v: Option<String>| v.unwrap_or_else(|| "Not configured".into());
    vec![
        ("policy_version", s.policy_version.clone()),
        ("timezone", s.timezone.clone()),
        ("evaluation_hour", s.evaluation_hour.to_string()),
        ("notifications_enabled", s.notifications_enabled.to_string()),
        (
            "savings_goal_contributions_enabled",
            s.savings_goal_contributions_enabled.to_string(),
        ),
        ("payday_day", or(s.payday_day.map(|d| d.to_string()))),
        (
            "monthly_salary_eur",
            or(s.monthly_salary_eur.map(|d| d.to_string())),
        ),
        ("salary_account_id", or(s.salary_account_id.clone())),
        ("safety_buffer_eur", s.safety_buffer_eur.to_string()),
        ("heads_up_spent_ratio", s.heads_up_spent_ratio.to_string()),
        ("heads_up_pace_ratio", s.heads_up_pace_ratio.to_string()),
        (
            "heads_up_cooldown_hours",
            s.heads_up_cooldown_hours.to_string(),
        ),
        (
            "warning_cooldown_hours",
            s.warning_cooldown_hours.to_string(),
        ),
        (
            "critical_cooldown_hours",
            s.critical_cooldown_hours.to_string(),
        ),
        (
            "forecast_excluded_categories",
            s.forecast_excluded_categories.join(","),
        ),
    ]
}

/// The form's fields as typed.
#[derive(Clone, Copy, Debug)]
struct Draft {
    payday: RwSignal<String>,
    salary: RwSignal<String>,
    buffer: RwSignal<String>,
    excluded: RwSignal<String>,
    salary_account: RwSignal<String>,
    timezone: RwSignal<String>,
    hour: RwSignal<String>,
    notifications: RwSignal<bool>,
    spent_ratio: RwSignal<String>,
    pace_ratio: RwSignal<String>,
    heads_up_hours: RwSignal<String>,
    warning_hours: RwSignal<String>,
    critical_hours: RwSignal<String>,
}

impl Draft {
    fn new() -> Self {
        let s = || RwSignal::new(String::new());
        Self {
            payday: s(),
            salary: s(),
            buffer: s(),
            excluded: s(),
            salary_account: s(),
            timezone: s(),
            hour: s(),
            notifications: RwSignal::new(false),
            spent_ratio: s(),
            pace_ratio: s(),
            heads_up_hours: s(),
            warning_hours: s(),
            critical_hours: s(),
        }
    }

    fn fill(self, s: &OwnerSettings) {
        self.payday
            .set(s.payday_day.map(|d| d.to_string()).unwrap_or_default());
        self.salary.set(
            s.monthly_salary_eur
                .map(|d| d.to_string())
                .unwrap_or_default(),
        );
        self.buffer.set(s.safety_buffer_eur.to_string());
        self.excluded.set(s.forecast_excluded_categories.join(", "));
        self.salary_account
            .set(s.salary_account_id.clone().unwrap_or_default());
        self.timezone.set(s.timezone.clone());
        self.hour.set(s.evaluation_hour.to_string());
        self.notifications.set(s.notifications_enabled);
        self.spent_ratio.set(s.heads_up_spent_ratio.to_string());
        self.pace_ratio.set(s.heads_up_pace_ratio.to_string());
        self.heads_up_hours
            .set(s.heads_up_cooldown_hours.to_string());
        self.warning_hours.set(s.warning_cooldown_hours.to_string());
        self.critical_hours
            .set(s.critical_cooldown_hours.to_string());
    }

    fn form(self) -> SettingsForm {
        SettingsForm {
            timezone: self.timezone.get_untracked(),
            evaluation_hour: self.hour.get_untracked(),
            notifications_enabled: self.notifications.get_untracked(),
            payday_day: self.payday.get_untracked(),
            monthly_salary_eur: self.salary.get_untracked(),
            salary_account_id: self.salary_account.get_untracked(),
            safety_buffer_eur: self.buffer.get_untracked(),
            heads_up_spent_ratio: self.spent_ratio.get_untracked(),
            heads_up_pace_ratio: self.pace_ratio.get_untracked(),
            heads_up_cooldown_hours: self.heads_up_hours.get_untracked(),
            warning_cooldown_hours: self.warning_hours.get_untracked(),
            critical_cooldown_hours: self.critical_hours.get_untracked(),
            forecast_excluded_categories: self
                .excluded
                .get_untracked()
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct FormArgs {
    form: SettingsForm,
}

#[derive(Serialize)]
struct ApplyArgs {
    apply: SettingsApply,
}

#[derive(Serialize)]
struct RollbackArgs {
    revision: i64,
}

#[component]
pub fn Settings() -> impl IntoView {
    let ops = Ops::new();
    let retry = RwSignal::new(0_u32);
    let workspace = load_workspace(retry);
    let draft = Draft::new();
    let preview = RwSignal::new(None::<SettingsPreview>);
    let rollback = RwSignal::new(None::<i64>);
    Effect::new(move |_| {
        if let Some(Ok(ws)) = workspace.data.get() {
            draft.fill(&ws.settings);
        }
    });
    view! {
        {frame(ops, workspace, retry, move |ws| {
            view! { <SettingsCard ws=ws ops=ops draft=draft preview=preview rollback=rollback /> }.into_any()
        })}
        <div class="mb-6 grid gap-4">
            <super::email::EmailCard />
            {move || workspace.data.get().and_then(Result::ok).map(|ws| view! { <LocalData ws=ws /> })}
            <Updates />
        </div>
    }
}

#[component]
fn SettingsCard(
    ws: ManageWorkspace,
    ops: Ops,
    draft: Draft,
    preview: RwSignal<Option<SettingsPreview>>,
    rollback: RwSignal<Option<i64>>,
) -> impl IntoView {
    let app = state();
    let busy = ops.busy;
    let notifications_available = ws.notifications_available;
    let mut accounts = vec![(String::new(), "Not set".to_owned())];
    accounts.extend(
        ws.accounts
            .iter()
            .map(|a| (a.account_id.clone(), a.name.clone())),
    );
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        ops.run(
            "preview_settings",
            FormArgs { form: draft.form() },
            move |p: SettingsPreview| preview.set(Some(p)),
        );
    };
    let thresholds = [
        ("heads_up_spent_ratio", draft.spent_ratio, "0", "1", "0.01"),
        (
            "heads_up_pace_ratio",
            draft.pace_ratio,
            "0.01",
            "10",
            "0.01",
        ),
        (
            "heads_up_cooldown_hours",
            draft.heads_up_hours,
            "1",
            "744",
            "1",
        ),
        (
            "warning_cooldown_hours",
            draft.warning_hours,
            "1",
            "744",
            "1",
        ),
        (
            "critical_cooldown_hours",
            draft.critical_hours,
            "1",
            "744",
            "1",
        ),
    ];
    let revision_count = ws.revisions.len();
    let revisions = ws.revisions.clone();
    view! {
        <Card>
            <Eyebrow level=Heading::P>"SETTINGS"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Finance preferences"</CardTitle>
            // Any edit invalidates the previewed change, as in the site.
            <form on:submit=submit on:input=move |_| preview.set(None)>
                <fieldset disabled=move || busy.get() class=FIELDSET>
                    <div class=GRID>
                        <Field label="Usual monthly payday" id="set-payday" hint="Shorter months use their final day.">
                            <Input id="set-payday" value=draft.payday kind="number" min="1" max="31" placeholder="Not configured" />
                        </Field>
                        <Field label="Monthly salary (EUR)" id="set-salary" hint="Expected income each payday; credited only when imported.">
                            <Input id="set-salary" value=draft.salary kind="number" min="0.01" step="0.01" />
                        </Field>
                        <Field label="Safety buffer (EUR)" id="set-buffer" hint="Kept on your spending accounts before month closure suggests saving the rest.">
                            <Input id="set-buffer" value=draft.buffer kind="number" min="0" step="0.01" />
                        </Field>
                        <Field
                            label="Not part of everyday spending"
                            id="set-excluded"
                            hint="Comma-separated categories of fixed or planned costs the everyday spending forecast leaves out."
                        >
                            <Input id="set-excluded" value=draft.excluded placeholder="e.g. Education, Flights" />
                        </Field>
                        <Field label="Salary account" id="set-salary-account" hint="The account your salary is paid into.">
                            <Select id="set-salary-account" value=draft.salary_account options=Signal::derive(move || accounts.clone()) />
                        </Field>
                        <Field label="Timezone" id="set-timezone">
                            <Input id="set-timezone" value=draft.timezone />
                        </Field>
                        <Field label="Evaluation hour" id="set-hour">
                            <Input id="set-hour" value=draft.hour kind="number" min="0" max="23" />
                        </Field>
                    </div>
                    <label class="mb-5 flex items-start gap-2.5 text-sm leading-relaxed">
                        <span class="mt-1 flex">
                            <Checkbox
                                checked=Signal::derive(move || draft.notifications.get())
                                disabled=Signal::derive(move || !notifications_available || busy.get())
                                on_change=Callback::new(move |v| {
                                    draft.notifications.set(v);
                                    preview.set(None);
                                })
                            />
                        </span>
                        <span>
                            "Desktop notifications "
                            {if notifications_available {
                                "— alerts about savings goals as macOS notifications"
                            } else {
                                "— delivery is not installed"
                            }}
                        </span>
                    </label>
                    <details class=cn([DETAILS, "mb-5"])>
                        <summary class=SUMMARY>"Alert thresholds & cooldowns"</summary>
                        <div class=GRID>
                            {thresholds
                                .into_iter()
                                .map(|(key, value, min, max, step)| {
                                    let id = format!("set-{key}");
                                    view! {
                                        <Field label=label(key) id=id.clone()>
                                            <Input id=id value=value kind="number" min=min max=max step=step />
                                        </Field>
                                    }
                                })
                                .collect_view()}
                        </div>
                        <p class="text-sm text-fg-muted">"Higher-severity alerts use shorter cooldowns."</p>
                    </details>
                    <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || busy.get())>
                        {move || if busy.get() { "Working…" } else { "Review changes" }}
                    </Button>
                </fieldset>
            </form>
            {move || {
                preview
                    .get()
                    .map(|p| {
                        let before = values(&p.before);
                        let rows = values(&p.after)
                            .into_iter()
                            .zip(before)
                            .map(|((key, after), (_, before))| {
                                view! {
                                    <tr class=TROW>
                                        <td class=TCELL>{label(key).to_owned()}</td>
                                        <td class=cn([TCELL, "text-fg-muted"])>{before}</td>
                                        <td class=TCELL>{after}</td>
                                    </tr>
                                }
                            })
                            .collect_view();
                        let apply = SettingsApply {
                            token: p.token.clone(),
                            settings: p.after.clone(),
                        };
                        view! {
                            <div class=cn([PREVIEW, "mt-5"])>
                                <h3 class="text-base font-semibold">"Review changes"</h3>
                                <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label="Settings changes">
                                    <table class=TABLE data-density="sm">
                                        <thead class="text-left">
                                            <tr>
                                                <th class=TH>"Setting"</th>
                                                <th class=TH>"Before"</th>
                                                <th class=TH>"After"</th>
                                            </tr>
                                        </thead>
                                        <tbody>{rows}</tbody>
                                    </table>
                                </div>
                                <div class="flex flex-wrap items-center gap-2">
                                    <Button
                                        disabled=Signal::derive(move || busy.get())
                                        on_click=Callback::new(move |_| {
                                            ops.run(
                                                "apply_settings",
                                                ApplyArgs { apply: apply.clone() },
                                                move |_: i64| {
                                                    preview.set(None);
                                                    ops.notice.set("Owner settings applied.".into());
                                                    app.refresh();
                                                },
                                            );
                                        })
                                    >
                                        "Save changes"
                                    </Button>
                                    <Button
                                        variant=Variant::Ghost
                                        disabled=Signal::derive(move || busy.get())
                                        on_click=Callback::new(move |_| preview.set(None))
                                    >
                                        "Cancel"
                                    </Button>
                                </div>
                            </div>
                        }
                    })
            }}
            <details class=cn([DETAILS, "mt-5 pb-0"])>
                <summary class=SUMMARY>{format!("Previous settings ({revision_count})")}</summary>
                {if revisions.is_empty() {
                    view! { <p class="pt-3 text-sm">"No saved revisions."</p> }.into_any()
                } else {
                    revisions
                        .into_iter()
                        .map(|r| {
                            let id = r.revision_id;
                            view! {
                                <div class="flex flex-wrap items-center justify-between gap-4 border-b border-line py-4 text-sm last:border-b-0">
                                    <span>
                                        {format!(
                                            "#{id} · {}{}",
                                            local_date(Some(&r.applied_at)),
                                            if r.rolled_back_at.is_some() { " · Rolled back" } else { "" },
                                        )}
                                    </span>
                                    {r
                                        .active
                                        .then(|| {
                                            view! {
                                                <Button
                                                    size=Size::Sm
                                                    variant=Variant::Secondary
                                                    disabled=Signal::derive(move || busy.get())
                                                    on_click=Callback::new(move |_| rollback.set(Some(id)))
                                                >
                                                    "Review rollback"
                                                </Button>
                                            }
                                        })}
                                    {move || {
                                        (rollback.get() == Some(id))
                                            .then(|| {
                                                view! {
                                                    <div class=cn([PREVIEW, "w-full"])>
                                                        <p class="text-sm">"Restore these settings?"</p>
                                                        <div class="flex flex-wrap items-center gap-2">
                                                            <Button
                                                                disabled=Signal::derive(move || busy.get())
                                                                on_click=Callback::new(move |_| {
                                                                    ops.run(
                                                                        "rollback_settings",
                                                                        RollbackArgs { revision: id },
                                                                        move |(): ()| {
                                                                            rollback.set(None);
                                                                            preview.set(None);
                                                                            ops.notice.set("Settings restored.".into());
                                                                            app.refresh();
                                                                        },
                                                                    );
                                                                })
                                                            >
                                                                "Confirm rollback"
                                                            </Button>
                                                            <Button
                                                                variant=Variant::Ghost
                                                                disabled=Signal::derive(move || busy.get())
                                                                on_click=Callback::new(move |_| rollback.set(None))
                                                            >
                                                                "Cancel"
                                                            </Button>
                                                        </div>
                                                    </div>
                                                }
                                            })
                                    }}
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </details>
        </Card>
    }
}

/// Where the data lives and what it covers.
#[component]
fn LocalData(ws: ManageWorkspace) -> impl IntoView {
    let covers = match (&ws.first_date, &ws.last_date) {
        (Some(a), Some(b)) => format!("{} – {}", format::date(a), format::date(b)),
        _ => "—".to_owned(),
    };
    let facts = [
        ("Transactions", format::count(ws.transactions)),
        ("Covers", covers),
        (
            "Last import",
            local_date(ws.last_import.as_ref().map(|r| r.finished_at.as_str())),
        ),
        ("Last write", local_date(ws.last_write.as_deref())),
    ];
    view! {
        <Card>
            <Eyebrow level=Heading::P>"THIS MAC"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Local data"</CardTitle>
            <dl class="mt-5 grid gap-4 sm:grid-cols-2 md:grid-cols-4">
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
            <p class="mt-4 text-xs text-fg-muted">
                "Stored at ~/Library/Application Support/Finance/finance.sqlite3. Everything stays on this Mac; the app backs the file up before the first change of each session."
            </p>
        </Card>
    }
}

#[derive(Serialize)]
struct TokenArgs {
    token: String,
}

/// App updates from the GitHub releases (moved from the old Data screen).
#[component]
fn Updates() -> impl IntoView {
    let changed = RwSignal::new(0_u32);
    let updates = load::<UpdateStatus, _>("update_status", move || {
        changed.track();
        NoArgs {}
    });
    // The backend announces every change in update state (check, install…).
    let on_change = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        changed.try_update(|n| *n += 1);
    });
    crate::api::listen("update-status", &on_change);
    on_change.forget();

    let ops = Ops::new();
    let act = move |cmd: &'static str| {
        ops.run(cmd, NoArgs {}, move |(): ()| {
            changed.update(|n| *n += 1);
        });
    };
    let token_input = NodeRef::<InputElement>::new();
    let save_token = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(el) = token_input.get() else { return };
        let token = el.value();
        el.set_value("");
        ops.run("set_release_token", TokenArgs { token }, move |(): ()| {
            changed.update(|n| *n += 1);
        });
    };
    let busy = ops.busy;
    let body = move |u: UpdateStatus| {
        let status_line = if u.installing {
            "Installing… the app restarts when done.".to_owned()
        } else if u.checking {
            "Checking for updates…".to_owned()
        } else if let Some(e) = &u.error {
            format!("Last check failed: {e}")
        } else if u.available.is_some() {
            "An update is ready.".to_owned()
        } else if let Some(t) = &u.last_checked {
            format!("Up to date · checked {t}")
        } else {
            "Not checked yet.".to_owned()
        };
        let failed = u.error.is_some();
        let (installing, checking) = (u.installing, u.checking);
        view! {
            <dl class="mt-5 grid gap-4 sm:grid-cols-3">
                <div>
                    <dt class="text-xs text-fg-muted">"Installed version"</dt>
                    <dd class="mt-1.5 text-sm tabular-nums">{u.current_version.clone()}</dd>
                </div>
                <div class="sm:col-span-2">
                    <dt class="text-xs text-fg-muted">"Status"</dt>
                    <dd class=cn(["mt-1.5 text-sm", if failed { "text-negative-fg" } else { "" }])>{status_line}</dd>
                </div>
            </dl>
            {u
                .available
                .clone()
                .map(|a| {
                    view! {
                        <div class=cn([PREVIEW, "mt-5 sm:grid-cols-[1fr_auto] sm:items-center"])>
                            <div class="grid gap-1">
                                <strong class="font-semibold">
                                    {format!("Version {} is available", a.version)}
                                    {a.date.map(|d| view! { <span class="font-normal text-fg-muted">{format!(" · {}", format::date(&d))}</span> })}
                                </strong>
                                {a.notes.filter(|n| !n.trim().is_empty()).map(|n| view! { <p class="whitespace-pre-line text-sm text-fg-muted">{n}</p> })}
                            </div>
                            <Button
                                disabled=Signal::derive(move || busy.get() || installing)
                                on_click=Callback::new(move |_| act("install_update"))
                            >
                                "Install and restart"
                            </Button>
                        </div>
                    }
                })}
            {if u.token_set {
                view! {
                    <div class="mt-5 flex flex-wrap items-center gap-2">
                        <Button
                            variant=Variant::Secondary
                            size=Size::Sm
                            disabled=Signal::derive(move || busy.get() || checking)
                            on_click=Callback::new(move |_| act("check_for_updates"))
                        >
                            "Check now"
                        </Button>
                        <Button
                            variant=Variant::Ghost
                            size=Size::Sm
                            disabled=Signal::derive(move || busy.get())
                            on_click=Callback::new(move |_| act("forget_release_token"))
                        >
                            "Remove release token"
                        </Button>
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="mt-5 grid gap-3">
                        <p class="text-sm text-fg-muted">
                            "Updates come from this app's GitHub releases. Paste a fine-grained GitHub token with read-only "
                            <em>"Contents"</em>
                            " access to the personal-finance-assistant repository. It is kept in your Keychain."
                        </p>
                        <form class="flex flex-col gap-3 sm:flex-row sm:items-end" on:submit=save_token>
                            <input
                                node_ref=token_input
                                class=cn([CONTROL, "sm:max-w-sm"])
                                type="password"
                                autocomplete="off"
                                placeholder="github_pat_…"
                                aria-label="GitHub release token"
                            />
                            <Button submit=true disabled=Signal::derive(move || busy.get())>
                                "Save and check"
                            </Button>
                        </form>
                    </div>
                }
                    .into_any()
            }}
        }
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"APP"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Updates"</CardTitle>
            {move || match updates.data.get() {
                None => view! { <p class="mt-5 text-sm text-fg-muted">"Checking update state…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p class="mt-5 text-sm text-fg-muted">{format!("Updates are not available here ({e}).")}</p> }
                        .into_any()
                }
                Some(Ok(u)) => body(u).into_any(),
            }}
            {move || {
                let e = ops.error.get();
                (!e.is_empty()).then(|| view! { <p role="alert" class=cn([super::ui::ERROR, "mt-4"])>{e}</p> })
            }}
        </Card>
    }
}
