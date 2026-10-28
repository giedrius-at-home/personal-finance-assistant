//! The Wishes section: planned purchases, whether the coming months can
//! afford them, and the credit card they may lean on.

use std::collections::HashMap;

use finance_types::gateway::{WishPurchase, WishSettings, WishSource, WishStatus};
use finance_types::report::Currency;
use finance_types::report::Dashboard;
use finance_types::report::wishes::{Cut, Verdict, WishMonth, WishRow, WishesView};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::IgnoredAny;

use crate::api::{NoArgs, invoke, load};
use crate::ds::{
    Button, CONTROL, Card, CardTone, Eyebrow, Field, Icon, Lucide, Size, Skeleton, Stat, Variant,
    cn,
};
use crate::pages::Problem;
use crate::{format, state};

/// An amount the way the verdicts say it: `€70`, `€40.02`.
fn amount(value: Decimal, code: &str) -> String {
    let decimals = if value.fract().is_zero() { 0 } else { 2 };
    format::money_dp(value, code, decimals)
}

/// `2026-11-01` → `November`.
fn month_name(month: &str) -> String {
    format::month_long(month)
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The ids whose verdicts belong to `row`: its own and its sub-wishes'.
fn row_ids(row: &WishRow) -> Vec<&str> {
    std::iter::once(row.wish.wish_id.as_str())
        .chain(row.children.iter().map(|c| c.wish.wish_id.as_str()))
        .collect()
}

/// One group per cut, in first-cut order, at its lowest amount.
fn merged_cuts<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for cut in verdicts.flat_map(|v| &v.cuts) {
        match out.iter_mut().find(|c| c.group_id == cut.group_id) {
            Some(seen) => seen.to = seen.to.min(cut.to),
            None => out.push(cut.clone()),
        }
    }
    out
}

/// The row's verdict in plain words: one sentence, the worst news first.
pub fn verdict_line(row: &WishRow, months: &[WishMonth]) -> String {
    let ids = row_ids(row);
    let mine = |v: &&Verdict| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id));
    let code = row.wish.currency.code.as_str();
    let short: Decimal = months
        .iter()
        .flat_map(|m| m.verdicts.iter().filter(mine))
        .map(|v| v.short)
        .sum();
    if short > Decimal::ZERO {
        return format!("Not affordable — short by {}.", amount(short, code));
    }
    let card = months.iter().find_map(|m| {
        let on_card: Decimal = m.verdicts.iter().filter(mine).map(|v| v.on_card).sum();
        (on_card > Decimal::ZERO).then_some((m, on_card))
    });
    if let Some((month, on_card)) = card {
        let next = month_name(&format::add_months(&month.month, 1));
        let n = amount(on_card, code);
        return format!("Affordable with {n} on the card — {next} starts {n} lighter.");
    }
    let cuts = months.iter().find_map(|m| {
        let cuts = merged_cuts(m.verdicts.iter().filter(mine));
        (!cuts.is_empty()).then_some(cuts)
    });
    if let Some(cuts) = cuts {
        let parts: Vec<String> = cuts
            .iter()
            .enumerate()
            .map(|(i, cut)| {
                let name = cut.name.to_lowercase();
                let to = amount(cut.to, code);
                if i == 0 {
                    format!("{name} drops to {to}")
                } else {
                    format!("{name} to {to}")
                }
            })
            .collect();
        return format!("Affordable if {}.", parts.join(" and "));
    }
    let drop = months.iter().find(|m| {
        m.free_left < m.free_before
            && m.verdicts
                .iter()
                .filter(mine)
                .any(|v| v.from_free > Decimal::ZERO)
    });
    if let Some(month) = drop {
        return format!(
            "Affordable — savings drop to {}.",
            amount(month.free_left, code)
        );
    }
    "Affordable.".to_owned()
}

/// `YYYY-MM-DD` of today, local time.
fn today() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    )
}

/// What `save_wish` takes (`WishDraft` only deserializes).
#[derive(Serialize)]
struct Draft {
    wish_id: Option<String>,
    parent_id: Option<String>,
    name: String,
    amount: Decimal,
    currency: Currency,
    due_on: String,
    source: WishSource,
}

#[derive(Serialize)]
struct DraftArgs {
    draft: Draft,
}

#[derive(Serialize)]
struct SavedArgs {
    wish_id: String,
    saved: Decimal,
}

#[derive(Serialize)]
struct WishIdArgs {
    wish_id: String,
}

#[derive(Serialize)]
struct BoughtArgs {
    wish_id: String,
    purchase: WishPurchase,
}

#[derive(Serialize)]
struct SettingsArgs {
    settings: WishSettings,
}

/// A source as a `<select>` value: `card` or `account:<id>`.
fn source_value(source: &WishSource) -> String {
    match source {
        WishSource::Card => "card".into(),
        WishSource::Account { account_id } => format!("account:{account_id}"),
    }
}

fn parse_source(value: &str) -> WishSource {
    match value.strip_prefix("account:") {
        Some(id) if !id.is_empty() => WishSource::Account {
            account_id: id.to_owned(),
        },
        _ => WishSource::Card,
    }
}

fn error_or(error: String, fallback: &str) -> String {
    if error.trim().is_empty() || error == "unknown-error" {
        fallback.to_owned()
    } else {
        error
    }
}

/// What the page reads from the dashboard.
#[derive(Clone, Debug, Default)]
struct Context {
    /// Spendable accounts: (id, name).
    accounts: Vec<(String, String)>,
    /// Bills: (id, name).
    bills: Vec<(String, String)>,
}

impl Context {
    fn from(report: &Dashboard) -> Self {
        let mut accounts: Vec<(String, String)> = Vec::new();
        for bank in report
            .monthly_allocation
            .iter()
            .flat_map(|m| &m.currencies)
            .flat_map(|c| &c.banks)
            .filter(|b| b.included)
        {
            if !accounts.iter().any(|(id, _)| *id == bank.account_id) {
                accounts.push((bank.account_id.clone(), bank.name.clone()));
            }
        }
        let mut bills: Vec<(String, String)> = Vec::new();
        for row in report
            .subscriptions
            .rows
            .iter()
            .chain(report.next_month_subscriptions.iter().flat_map(|s| &s.rows))
        {
            if !bills.iter().any(|(id, _)| *id == row.bill_id) {
                bills.push((row.bill_id.clone(), row.name.clone()));
            }
        }
        Self { accounts, bills }
    }

    fn source_label(&self, source: &WishSource) -> String {
        match source {
            WishSource::Card => "Credit card".into(),
            WishSource::Account { account_id } => self
                .accounts
                .iter()
                .find(|(id, _)| id == account_id)
                .map_or_else(|| "Bank account".into(), |(_, name)| name.clone()),
        }
    }

    fn source_options(&self) -> Vec<(String, String)> {
        std::iter::once(("card".to_owned(), "Credit card".to_owned()))
            .chain(
                self.accounts
                    .iter()
                    .map(|(id, name)| (format!("account:{id}"), name.clone())),
            )
            .collect()
    }
}

/// Screen state that outlives a reload.
#[derive(Clone, Copy)]
struct Ui {
    app: crate::AppState,
    busy: RwSignal<bool>,
    /// Feedback per target: (text, is error).
    messages: RwSignal<HashMap<String, (String, bool)>>,
    name: RwSignal<String>,
    amount: RwSignal<String>,
    due_on: RwSignal<String>,
    source: RwSignal<String>,
    parent: RwSignal<String>,
    /// The wish whose "Bought" form is open.
    buying: RwSignal<Option<String>>,
    buy_amount: RwSignal<String>,
    buy_on: RwSignal<String>,
    buy_source: RwSignal<String>,
    /// The wish whose drop is being confirmed.
    dropping: RwSignal<Option<String>>,
}

impl Ui {
    fn new() -> Self {
        Self {
            app: state(),
            busy: RwSignal::new(false),
            messages: RwSignal::new(HashMap::new()),
            name: RwSignal::new(String::new()),
            amount: RwSignal::new(String::new()),
            due_on: RwSignal::new(String::new()),
            source: RwSignal::new("card".into()),
            parent: RwSignal::new(String::new()),
            buying: RwSignal::new(None),
            buy_amount: RwSignal::new(String::new()),
            buy_on: RwSignal::new(String::new()),
            buy_source: RwSignal::new(String::new()),
            dropping: RwSignal::new(None),
        }
    }

    fn set_message(self, target: &str, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.messages.update(|m| {
            m.insert(target.to_owned(), (text, error));
        });
    }

    /// Run a write command; on success show `done` and reload every screen.
    fn write<A: Serialize + 'static>(
        self,
        target: String,
        cmd: &'static str,
        args: A,
        done: &'static str,
        after: impl FnOnce() + 'static,
    ) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.set_message(&target, "", false);
        spawn_local(async move {
            match invoke::<IgnoredAny>(cmd, &args).await {
                Ok(_) => {
                    self.set_message(&target, done, false);
                    after();
                    self.app.refresh();
                }
                Err(e) => self.set_message(&target, error_or(e, "Could not save."), true),
            }
            self.busy.set(false);
        });
    }
}

/// The receipt or error of `target`, when there is one.
fn feedback(ui: Ui, target: String) -> impl IntoView {
    move || {
        ui.messages.with(|m| {
            m.get(&target).filter(|(text, _)| !text.is_empty()).map(|(text, error)| {
                let class = cn([
                    "mt-2 break-words text-sm",
                    if *error { "text-negative-fg" } else { "text-positive-fg" },
                ]);
                view! { <p role=if *error { "alert" } else { "status" } class=class>{text.clone()}</p> }
            })
        })
    }
}

fn select(
    id: String,
    value: RwSignal<String>,
    options: Vec<(String, String)>,
) -> impl IntoView + use<> {
    let options = options
        .into_iter()
        .map(|(v, l)| {
            let selected = v == value.get_untracked();
            view! { <option value=v selected=selected>{l}</option> }
        })
        .collect_view();
    view! {
        <div class="relative w-full">
            <select
                id=id
                class=cn([CONTROL, "appearance-none pr-9"])
                prop:value=move || value.get()
                on:change=move |e| value.set(event_target_value(&e))
            >
                {options}
            </select>
            <span aria-hidden="true" class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

#[component]
pub fn Wishes() -> impl IntoView {
    let ui = Ui::new();
    let revision = ui.app.revision;
    let view_data = load::<WishesView, _>("wishes", move || {
        revision.track();
        NoArgs {}
    });
    move || {
        let ctx = ui.app.report.data.with(|d| {
            d.as_ref()
                .and_then(|r| r.as_ref().ok())
                .map(Context::from)
                .unwrap_or_default()
        });
        view_data.data.with(|data| match data {
            None => view! {
                <div class="grid gap-4">
                    <Skeleton class="h-32 rounded-xl" />
                    <Skeleton class="h-56 rounded-xl" />
                </div>
            }
            .into_any(),
            Some(Err(e)) => view! { <Problem error=e.clone() /> }.into_any(),
            Some(Ok(view)) => page(ui, view, &ctx).into_any(),
        })
    }
}

fn page(ui: Ui, view: &WishesView, ctx: &Context) -> impl IntoView + use<> {
    let current_month = view
        .months
        .first()
        .map_or_else(|| format!("{}-01", &today()[..7]), |m| m.month.clone());
    let months = StoredValue::new(view.months.clone());
    let ctx = StoredValue::new(ctx.clone());
    let currency = view.currency.clone();
    let (open, done): (Vec<WishRow>, Vec<WishRow>) = view
        .wishes
        .iter()
        .cloned()
        .partition(|r| r.wish.status == WishStatus::Planned);
    let parents: Vec<(String, String)> = open
        .iter()
        .filter(|r| r.wish.parent_id.is_none())
        .map(|r| (r.wish.wish_id.clone(), r.wish.name.clone()))
        .collect();
    let empty = open.is_empty();
    let rows = open
        .into_iter()
        .map(|row| wish_row(ui, row, months, ctx))
        .collect_view();
    let done_count = done.len();
    let done_rows = done
        .into_iter()
        .map(|row| done_row(&row, &ctx.get_value()))
        .collect_view();
    let month_cards = view
        .months
        .iter()
        .map(|m| month_card(m, &currency.code))
        .collect_view();
    let has_months = !view.months.is_empty();
    let strip = card_strip(ui, view, &ctx.get_value(), current_month);
    let form = add_form(ui, &ctx.get_value(), parents, currency);
    view! {
        <div class="grid gap-4">
            {strip}
            <Card>
                <Eyebrow>"NEW WISH"</Eyebrow>
                <h2 class="mt-2 text-lg font-semibold">"Add a wish"</h2>
                {form}
            </Card>
            <Card>
                <Eyebrow>"WISHES"</Eyebrow>
                <h2 class="mt-2 text-lg font-semibold">"Planned purchases"</h2>
                {feedback(ui, "dropped".into())}
                <div class="mt-3 grid">{rows}</div>
                {empty.then(|| view! { <p class="mt-3 text-sm text-fg-muted">"No planned wishes yet."</p> })}
                {(done_count > 0).then(|| view! {
                    <details class="mt-4 border-t border-line pt-3 text-sm">
                        <summary class="cursor-pointer text-fg-muted">{format!("Done ({done_count})")}</summary>
                        <div class="mt-2 grid gap-1 text-fg-muted">{done_rows}</div>
                    </details>
                })}
            </Card>
            {has_months.then(|| view! {
                <div>
                    <Eyebrow level=crate::ds::Heading::H2>"MONTHS"</Eyebrow>
                    <div class="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">{month_cards}</div>
                </div>
            })}
        </div>
    }
}

fn card_strip(
    ui: Ui,
    view: &WishesView,
    ctx: &Context,
    current_month: String,
) -> impl IntoView + use<> {
    let settings = view.settings.clone();
    let limit = RwSignal::new(settings.card_limit.to_string());
    let used = RwSignal::new(settings.card_used.to_string());
    let bill = RwSignal::new(settings.card_bill_id.clone().unwrap_or_default());
    let bills: Vec<(String, String)> = std::iter::once((String::new(), "No card bill".to_owned()))
        .chain(ctx.bills.iter().cloned())
        .collect();
    let stale = view.card_stale.then(|| {
        let entered = if settings.card_used_month.is_empty() {
            "an earlier month".to_owned()
        } else {
            format::month_long(&settings.card_used_month)
        };
        view! {
            <p role="status" class="mt-2 text-sm text-warning-fg">
                {format!("The card usage was entered for {entered}. Enter this month's so far; until then it counts as €0.")}
            </p>
        }
    });
    let month_label = format::month_long(&current_month);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let (Ok(card_limit), Ok(card_used)) = (
            limit.get_untracked().trim().parse::<Decimal>(),
            used.get_untracked().trim().parse::<Decimal>(),
        ) else {
            ui.set_message("card", "Enter the limit and the amount used.", true);
            return;
        };
        let bill_id = bill.get_untracked();
        let settings = WishSettings {
            card_limit,
            card_used,
            card_used_month: current_month.clone(),
            card_bill_id: (!bill_id.is_empty()).then_some(bill_id),
        };
        ui.write(
            "card".into(),
            "save_wish_settings",
            SettingsArgs { settings },
            "Card saved.",
            || {},
        );
    };
    view! {
        <Card>
            <Eyebrow>"CREDIT CARD"</Eyebrow>
            <h2 class="mt-2 text-lg font-semibold">"What the wishes may lean on"</h2>
            {stale}
            <form class="mt-4 grid items-end gap-3 sm:grid-cols-2 lg:grid-cols-4" on:submit=submit>
                <Field label="Limit" id="card-limit">
                    <input
                        id="card-limit"
                        type="number"
                        min="0.01"
                        step="0.01"
                        required
                        class=CONTROL
                        prop:value=move || limit.get()
                        on:input=move |e| limit.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Used this month" id="card-used" hint=format!("So far in {month_label}.")>
                    <input
                        id="card-used"
                        type="number"
                        min="0"
                        step="0.01"
                        required
                        class=CONTROL
                        prop:value=move || used.get()
                        on:input=move |e| used.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Card bill" id="card-bill" hint="The bill the card is paid by.">
                    {select("card-bill".into(), bill, bills)}
                </Field>
                <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || ui.busy.get())>
                    "Save card"
                </Button>
            </form>
            {feedback(ui, "card".into())}
        </Card>
    }
}

fn add_form(
    ui: Ui,
    ctx: &Context,
    parents: Vec<(String, String)>,
    currency: Currency,
) -> impl IntoView + use<> {
    let sources = ctx.source_options();
    let parent_options: Vec<(String, String)> =
        std::iter::once((String::new(), "Nothing".to_owned()))
            .chain(parents)
            .collect();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(amount) = ui.amount.get_untracked().trim().parse::<Decimal>() else {
            ui.set_message("add", "Enter an amount.", true);
            return;
        };
        let parent = ui.parent.get_untracked();
        let draft = Draft {
            wish_id: None,
            parent_id: (!parent.is_empty()).then_some(parent),
            name: ui.name.get_untracked().trim().to_owned(),
            amount,
            currency: currency.clone(),
            due_on: ui.due_on.get_untracked(),
            source: parse_source(&ui.source.get_untracked()),
        };
        ui.write(
            "add".into(),
            "save_wish",
            DraftArgs { draft },
            "Wish added.",
            move || {
                for field in [ui.name, ui.amount, ui.due_on, ui.parent] {
                    field.set(String::new());
                }
            },
        );
    };
    view! {
        <form class="mt-4 grid items-end gap-3 sm:grid-cols-2 lg:grid-cols-3" on:submit=submit>
            <Field label="Name" id="wish-name">
                <input
                    id="wish-name"
                    required
                    maxlength="128"
                    placeholder="e.g. Winter jacket"
                    class=CONTROL
                    prop:value=move || ui.name.get()
                    on:input=move |e| ui.name.set(event_target_value(&e))
                />
            </Field>
            <Field label="Amount" id="wish-amount" hint="For a wish with parts, the most it may cost.">
                <input
                    id="wish-amount"
                    type="number"
                    min="0.01"
                    step="0.01"
                    required
                    placeholder="0.00"
                    class=CONTROL
                    prop:value=move || ui.amount.get()
                    on:input=move |e| ui.amount.set(event_target_value(&e))
                />
            </Field>
            <Field label="Date" id="wish-date">
                <input
                    id="wish-date"
                    type="date"
                    required
                    class=CONTROL
                    prop:value=move || ui.due_on.get()
                    on:input=move |e| ui.due_on.set(event_target_value(&e))
                />
            </Field>
            <Field label="Pay with" id="wish-source">
                {select("wish-source".into(), ui.source, sources)}
            </Field>
            <Field label="Part of" id="wish-parent">
                {select("wish-parent".into(), ui.parent, parent_options)}
            </Field>
            <Button submit=true variant=Variant::Secondary disabled=Signal::derive(move || ui.busy.get())>
                <Lucide icon=Icon::Plus size=15 />
                "Add wish"
            </Button>
        </form>
        {feedback(ui, "add".into())}
    }
}

/// A top-level planned wish, with its sub-wishes.
fn wish_row(
    ui: Ui,
    row: WishRow,
    months: StoredValue<Vec<WishMonth>>,
    ctx: StoredValue<Context>,
) -> impl IntoView + use<> {
    let code = row.wish.currency.code.clone();
    let parent = !row.children.is_empty();
    let family = parent.then(|| {
        let over = row.over_cap.then(|| view! {
            <p role="status" class="text-sm text-warning-fg">"The parts add up to more than the cap."</p>
        });
        view! {
            <p class="text-sm text-fg-muted">
                {format!("Cap {} · rest {}", amount(row.wish.amount, &code), amount(row.rest, &code))}
            </p>
            {over}
        }
    });
    let set_aside = (row.monthly_set_aside > Decimal::ZERO).then(|| {
        view! {
            <p class="text-sm">{format!("Put aside {}/month", amount(row.monthly_set_aside, &code))}</p>
        }
    });
    let children = row
        .children
        .iter()
        .map(|child| {
            if child.wish.status == WishStatus::Planned {
                view! {
                    <div class="border-t border-line pl-4 sm:pl-6">
                        {wish_body(ui, child, months, ctx, true)}
                    </div>
                }
                .into_any()
            } else {
                view! { <div class="border-t border-line py-2 pl-4 text-sm text-fg-muted sm:pl-6">{done_row(child, &ctx.get_value())}</div> }
                    .into_any()
            }
        })
        .collect_view();
    let body = wish_body(ui, &row, months, ctx, false);
    view! {
        <div class="border-t border-line first:border-t-0">
            {body}
            <div class="grid gap-1 pb-3">
                {family}
                {set_aside}
            </div>
            {children}
        </div>
    }
}

/// Name, amount, date, source, verdict and actions of one planned wish.
fn wish_body(
    ui: Ui,
    row: &WishRow,
    months: StoredValue<Vec<WishMonth>>,
    ctx: StoredValue<Context>,
    sub: bool,
) -> impl IntoView + use<> {
    let wish = row.wish.clone();
    let code = wish.currency.code.clone();
    let line = months.with_value(|m| verdict_line(row, m));
    let tone = if row.short > Decimal::ZERO {
        "text-negative-fg"
    } else if row.tradeoffs {
        "text-warning-fg"
    } else {
        "text-positive-fg"
    };
    let detail = format!(
        "{} · by {} · {}",
        amount(wish.amount, &code),
        format::date(&wish.due_on),
        ctx.with_value(|c| c.source_label(&wish.source))
    );
    let target = format!("wish:{}", wish.wish_id);
    let saved = RwSignal::new(wish.saved.to_string());
    let id = StoredValue::new(wish.wish_id.clone());
    let save_saved = {
        let target = target.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            let Ok(value) = saved.get_untracked().trim().parse::<Decimal>() else {
                ui.set_message(&target, "Enter the amount saved.", true);
                return;
            };
            ui.write(
                target.clone(),
                "record_wish_saved",
                SavedArgs {
                    wish_id: id.get_value(),
                    saved: value,
                },
                "Saved amount updated.",
                || {},
            );
        }
    };
    let open_buy = {
        let wish = wish.clone();
        move |_| {
            ui.dropping.set(None);
            ui.buy_amount.set(wish.amount.to_string());
            ui.buy_on.set(today());
            ui.buy_source.set(source_value(&wish.source));
            ui.buying.set(Some(wish.wish_id.clone()));
        }
    };
    let buying = move || ui.buying.get().as_deref() == Some(id.get_value().as_str());
    let dropping = move || ui.dropping.get().as_deref() == Some(id.get_value().as_str());
    let sources = ctx.with_value(Context::source_options);
    let buy_target = target.clone();
    let buy_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = ui.buy_amount.get_untracked().trim().parse::<Decimal>() else {
            ui.set_message(&buy_target, "Enter what it cost.", true);
            return;
        };
        let purchase = WishPurchase {
            amount: value,
            on: ui.buy_on.get_untracked(),
            source: parse_source(&ui.buy_source.get_untracked()),
        };
        ui.write(
            buy_target.clone(),
            "mark_wish_bought",
            BoughtArgs {
                wish_id: id.get_value(),
                purchase,
            },
            "Marked as bought.",
            move || ui.buying.set(None),
        );
    };
    let confirm_drop = move |_| {
        ui.write(
            "dropped".into(),
            "drop_wish",
            WishIdArgs {
                wish_id: id.get_value(),
            },
            "Wish dropped.",
            move || ui.dropping.set(None),
        );
    };
    let saved_id = format!("saved-{}", wish.wish_id);
    let buy_amount_id = StoredValue::new(format!("buy-amount-{}", wish.wish_id));
    let buy_on_id = StoredValue::new(format!("buy-on-{}", wish.wish_id));
    let buy_source_id = StoredValue::new(format!("buy-source-{}", wish.wish_id));
    let drop_text = format!("Drop {}? It stops counting against your months.", wish.name);
    let buy_sources = StoredValue::new(sources);
    view! {
        <div class="grid gap-2 py-3">
            <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <h3 class=if sub { "text-sm font-semibold" } else { "text-base font-semibold" }>{wish.name.clone()}</h3>
                <span class="text-sm tabular-nums text-fg-muted">{detail}</span>
            </div>
            <p class=cn(["text-sm font-medium", tone])>{line}</p>
            <div class="flex flex-wrap items-end gap-2">
                <form class="flex items-end gap-2" on:submit=save_saved>
                    <div class="grid gap-1">
                        <label for=saved_id.clone() class="text-xs text-fg-muted">"Saved so far"</label>
                        <input
                            id=saved_id
                            type="number"
                            min="0"
                            step="0.01"
                            class=cn([CONTROL, "h-8 w-28"])
                            prop:value=move || saved.get()
                            on:input=move |e| saved.set(event_target_value(&e))
                        />
                    </div>
                    <Button submit=true size=Size::Sm variant=Variant::Secondary disabled=Signal::derive(move || ui.busy.get())>
                        "Save"
                    </Button>
                </form>
                <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_buy)>
                    "Bought"
                </Button>
                <Button
                    size=Size::Sm
                    variant=Variant::Ghost
                    on_click=Callback::new(move |_| {
                        ui.buying.set(None);
                        ui.dropping.set(Some(id.get_value()));
                    })
                >
                    "Drop"
                </Button>
            </div>
            <Show when=buying>
                <form class="grid items-end gap-2 sm:grid-cols-4" on:submit=buy_submit.clone()>
                    <Field label="Cost" id=buy_amount_id.get_value()>
                        <input
                            id=buy_amount_id.get_value()
                            type="number"
                            min="0.01"
                            step="0.01"
                            required
                            class=CONTROL
                            prop:value=move || ui.buy_amount.get()
                            on:input=move |e| ui.buy_amount.set(event_target_value(&e))
                        />
                    </Field>
                    <Field label="On" id=buy_on_id.get_value()>
                        <input
                            id=buy_on_id.get_value()
                            type="date"
                            required
                            class=CONTROL
                            prop:value=move || ui.buy_on.get()
                            on:input=move |e| ui.buy_on.set(event_target_value(&e))
                        />
                    </Field>
                    <Field label="Paid with" id=buy_source_id.get_value()>
                        {select(buy_source_id.get_value(), ui.buy_source, buy_sources.get_value())}
                    </Field>
                    <div class="flex gap-2">
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Mark bought"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.buying.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=dropping>
                <div class="flex flex-wrap items-center gap-2 text-sm">
                    <span>{drop_text.clone()}</span>
                    <Button size=Size::Sm variant=Variant::Danger on_click=Callback::new(confirm_drop)>"Drop wish"</Button>
                    <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| ui.dropping.set(None))>"Keep"</Button>
                </div>
            </Show>
            {feedback(ui, target)}
        </div>
    }
}

/// A bought or dropped wish, in one muted line.
fn done_row(row: &WishRow, ctx: &Context) -> impl IntoView + use<> {
    let wish = &row.wish;
    let code = &wish.currency.code;
    let text = match (&wish.status, &wish.bought) {
        (WishStatus::Bought, Some(p)) => format!(
            "{} · bought {} on {} · {}",
            wish.name,
            amount(p.amount, code),
            format::date(&p.on),
            ctx.source_label(&p.source)
        ),
        (WishStatus::Bought, None) => format!("{} · bought", wish.name),
        (WishStatus::Started, _) => format!("{} · started", wish.name),
        _ => format!("{} · dropped · {}", wish.name, amount(wish.amount, code)),
    };
    view! { <p class="py-1">{text}</p> }
}

fn month_card(month: &WishMonth, code: &str) -> impl IntoView + use<> {
    let unit = Currency::new(code);
    let money = |v: Decimal| crate::ds::money(v, &unit);
    let title = format::month_long(&month.month);
    let (before, after, charge, used) = (
        money(month.free_before),
        money(month.free_left),
        money(month.card_charge_in),
        money(month.card_used),
    );
    view! {
        <Card tone=CardTone::Muted class="grid gap-3 p-4 sm:p-5">
            <h3 class="text-sm font-semibold">{title}</h3>
            <div class="grid grid-cols-2 gap-3">
                <Stat label="Free before">{before}</Stat>
                <Stat label="Free after">{after}</Stat>
                <Stat label="Card charge in">{charge}</Stat>
                <Stat label="Card used">{used}</Stat>
            </div>
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::gateway::{Wish, WishSource, WishStatus};

    fn d(n: i64, scale: u32) -> Decimal {
        Decimal::new(n, scale)
    }

    fn row(id: &str) -> WishRow {
        WishRow {
            wish: Wish {
                wish_id: id.into(),
                parent_id: None,
                name: "Jacket".into(),
                amount: d(150, 0),
                currency: Currency::new("EUR"),
                due_on: "2026-10-20".into(),
                source: WishSource::Card,
                saved: Decimal::ZERO,
                status: WishStatus::Planned,
                bought: None,
                kind: Default::default(),
                created_at: "2026-10-01T00:00:00Z".into(),
            },
            children: Vec::new(),
            rest: Decimal::ZERO,
            over_cap: false,
            monthly_set_aside: Decimal::ZERO,
            short: Decimal::ZERO,
            tradeoffs: false,
        }
    }

    fn verdict(id: &str, amount: Decimal) -> Verdict {
        Verdict {
            wish_id: Some(id.into()),
            label: "Jacket".into(),
            amount,
            from_free: Decimal::ZERO,
            cuts: Vec::new(),
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
        }
    }

    fn month(free_before: Decimal, free_left: Decimal, verdicts: Vec<Verdict>) -> WishMonth {
        WishMonth {
            month: "2026-10-01".into(),
            free_before,
            free_left,
            card_charge_in: Decimal::ZERO,
            card_used: d(998, 2),
            verdicts,
        }
    }

    fn cut(id: &str, name: &str, from: i64, to: i64) -> Cut {
        Cut {
            group_id: id.into(),
            name: name.into(),
            from: d(from, 0),
            to: d(to, 0),
        }
    }

    #[test]
    fn fits_in_free_money_says_savings_drop() {
        let v = Verdict {
            from_free: d(150, 0),
            ..verdict("w", d(150, 0))
        };
        let months = [month(d(500, 0), d(350, 0), vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable — savings drop to €350."
        );
    }

    #[test]
    fn savings_drop_keeps_cents() {
        let v = Verdict {
            from_free: d(5998, 2),
            ..verdict("w", d(5998, 2))
        };
        let months = [month(d(100, 0), d(4002, 2), vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable — savings drop to €40.02."
        );
    }

    #[test]
    fn cuts_name_going_out_then_groceries() {
        let v = Verdict {
            cuts: vec![
                cut("g1", "Going out", 120, 0),
                cut("g2", "Groceries", 450, 370),
            ],
            ..verdict("w", d(200, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable if going out drops to €0 and groceries to €370."
        );
    }

    #[test]
    fn card_use_names_the_next_month() {
        let v = Verdict {
            cuts: vec![
                cut("g1", "Going out", 120, 0),
                cut("g2", "Groceries", 450, 300),
            ],
            on_card: d(130, 0),
            ..verdict("w", d(400, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable with €130 on the card — November starts €130 lighter."
        );
    }

    #[test]
    fn short_wins_over_everything() {
        let v = Verdict {
            short: d(50, 0),
            on_card: d(10, 0),
            ..verdict("w", d(50, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Not affordable — short by €50."
        );
    }

    #[test]
    fn nothing_taken_is_plainly_affordable() {
        let other = Verdict {
            from_free: d(80, 0),
            ..verdict("other", d(80, 0))
        };
        let months = [month(d(100, 0), d(20, 0), vec![other])];
        assert_eq!(verdict_line(&row("w"), &months), "Affordable.");
    }

    #[test]
    fn parent_counts_its_sub_wishes() {
        let mut parent = row("trip");
        parent.children.push(row("flights"));
        let v = Verdict {
            on_card: d(900, 0),
            ..verdict("flights", d(900, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&parent, &months),
            "Affordable with €900 on the card — November starts €900 lighter."
        );
    }
}
