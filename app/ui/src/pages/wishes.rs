//! The Wishes section: planned purchases, whether the coming months can
//! afford them, and the credit card they may lean on.

use std::collections::HashMap;

use finance_types::gateway::{
    Repeat, Wish, WishKind, WishPurchase, WishSettings, WishSource, WishStatus,
};
use finance_types::report::Currency;
use finance_types::report::Dashboard;
use finance_types::report::wishes::{ChangeableBill, Cut, Verdict, WishMonth, WishRow, WishesView};
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

/// What an amount box accepts: a plain amount with a point or a comma.
const AMOUNT_PATTERN: &str = "[0-9]+([.,][0-9]{1,2})?";

/// Drops a control that sits beside fields by its label's height
/// (`text-sm` line plus the field's gap), so it lines up with their boxes.
const LABEL_OFFSET: &str = "sm:mt-[26px]";

/// Reads an amount box; a comma counts as the decimal point.
fn parse_amount(value: &str) -> Result<Decimal, rust_decimal::Error> {
    value.trim().replace(',', ".").parse()
}

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
fn merged_cuts<'a>(cuts: impl Iterator<Item = &'a Cut>) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for cut in cuts {
        match out.iter_mut().find(|c| c.group_id == cut.group_id) {
            Some(seen) => seen.to = seen.to.min(cut.to),
            None => out.push(cut.clone()),
        }
    }
    out
}

/// `going out drops to €0 and groceries to €300`.
fn cuts_phrase(cuts: &[Cut], code: &str) -> String {
    cuts.iter()
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
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The row's verdicts month by month, from the first month that has one.
fn row_months<'a>(
    row: &WishRow,
    months: &'a [WishMonth],
) -> Vec<(&'a WishMonth, Vec<&'a Verdict>)> {
    let ids = row_ids(row);
    months
        .iter()
        .map(|m| {
            let mine = m
                .verdicts
                .iter()
                .filter(|v| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id)))
                .collect::<Vec<_>>();
            (m, mine)
        })
        .filter(|(_, mine)| !mine.is_empty())
        .collect()
}

fn total(verdicts: &[&Verdict], part: impl Fn(&Verdict) -> Decimal) -> Decimal {
    verdicts.iter().map(|v| part(v)).sum()
}

/// The merged cuts of the first month that has any.
fn first_cuts(by_month: &[(&WishMonth, Vec<&Verdict>)]) -> Vec<Cut> {
    by_month
        .iter()
        .map(|(_, mine)| merged_cuts(mine.iter().flat_map(|v| &v.cuts)))
        .find(|cuts| !cuts.is_empty())
        .unwrap_or_default()
}

/// The row's verdict in plain words: one sentence, the worst news first.
pub fn verdict_line(row: &WishRow, months: &[WishMonth]) -> String {
    if let Some(why) = &row.not_judged {
        return format!("Not judged yet: {why}.");
    }
    let by_month = row_months(row, months);
    let code = row.wish.currency.code.as_str();
    if row.wish.kind == WishKind::OneOff {
        one_off_line(&by_month, code)
    } else {
        repeating_line(&by_month, code)
    }
}

fn one_off_line(by_month: &[(&WishMonth, Vec<&Verdict>)], code: &str) -> String {
    let short: Decimal = by_month.iter().map(|(_, v)| total(v, |v| v.short)).sum();
    if short > Decimal::ZERO {
        return format!("Not affordable — short by {}.", amount(short, code));
    }
    let cuts = first_cuts(by_month);
    let on_card: Decimal = by_month.iter().map(|(_, v)| total(v, |v| v.on_card)).sum();
    let first_card = by_month
        .iter()
        .find(|(_, v)| total(v, |v| v.on_card) > Decimal::ZERO);
    if let (Some((month, _)), true) = (first_card, on_card > Decimal::ZERO) {
        let next = month_name(&format::add_months(&month.month, 1));
        let n = amount(on_card, code);
        let next_short: Decimal = by_month
            .iter()
            .map(|(_, v)| total(v, |v| v.next_short))
            .sum();
        let next_cuts = merged_cuts(
            by_month
                .iter()
                .flat_map(|(_, v)| v.iter())
                .flat_map(|v| &v.next_cuts),
        );
        if next_short > Decimal::ZERO {
            let even = if next_cuts.is_empty() {
                String::new()
            } else {
                format!(" even after {}", cuts_phrase(&next_cuts, code))
            };
            return format!(
                "Not affordable — {n} on the card, so {next} starts {} short{even}.",
                amount(next_short, code)
            );
        }
        let there = if next_cuts.is_empty() {
            String::new()
        } else {
            format!(", so {} in {next}", cuts_phrase(&next_cuts, code))
        };
        let after = if cuts.is_empty() {
            String::new()
        } else {
            format!(" — after {}", cuts_phrase(&cuts, code))
        };
        return format!(
            "Affordable with {n} on the card — {next} starts {n} lighter{there}{after}."
        );
    }
    if !cuts.is_empty() {
        return format!("Affordable if {}.", cuts_phrase(&cuts, code));
    }
    let drop = by_month.iter().find(|(m, v)| {
        m.free_left < m.free_before && v.iter().any(|v| v.from_free > Decimal::ZERO)
    });
    if let Some((month, _)) = drop {
        return format!(
            "Affordable — savings drop to {}.",
            amount(month.free_left, code)
        );
    }
    "Affordable.".to_owned()
}

/// Each month's short (`YYYY-MM`), in month order, with a card charge's
/// short counted in the month that pays the charge.
fn shorts_by_month(by_month: &[(&WishMonth, Vec<&Verdict>)]) -> Vec<(String, Decimal)> {
    let mut out: Vec<(String, Decimal)> = by_month
        .iter()
        .map(|(m, v)| {
            let key = m.month.get(..7).unwrap_or(&m.month).to_owned();
            (key, total(v, |v| v.short))
        })
        .collect();
    for (m, v) in by_month {
        let next_short = total(v, |v| v.next_short);
        if next_short > Decimal::ZERO {
            let next = format::add_months(&m.month, 1);
            match out.iter_mut().find(|(k, _)| *k == next) {
                Some((_, n)) => *n += next_short,
                None => out.push((next, next_short)),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A recurring wish or bill change, judged over every month it touches.
fn repeating_line(by_month: &[(&WishMonth, Vec<&Verdict>)], code: &str) -> String {
    let shorts = shorts_by_month(by_month);
    if let Some(i) = shorts.iter().position(|(_, n)| *n > Decimal::ZERO) {
        let (month, n) = &shorts[i];
        let n = amount(*n, code);
        let from = month_name(month);
        return match i.checked_sub(1) {
            None => format!("Not affordable — short by {n} from {from}."),
            Some(j) => format!(
                "Fits until {}. From {from}, short by {n}.",
                month_name(&shorts[j].0)
            ),
        };
    }
    let freed = by_month.iter().find_map(|(m, v)| {
        let sum = total(v, |v| v.amount);
        (sum < Decimal::ZERO).then_some((m, -sum))
    });
    if let Some((month, n)) = freed {
        return format!(
            "Frees {} a month from {}.",
            amount(n, code),
            month_name(&month.month)
        );
    }
    let mut cuts = first_cuts(by_month);
    if cuts.is_empty() {
        cuts = by_month
            .iter()
            .map(|(_, mine)| merged_cuts(mine.iter().flat_map(|v| &v.next_cuts)))
            .find(|c| !c.is_empty())
            .unwrap_or_default();
    }
    let card = by_month
        .iter()
        .map(|(_, v)| total(v, |v| v.on_card))
        .find(|n| *n > Decimal::ZERO);
    if let Some(n) = card {
        let after = if cuts.is_empty() {
            String::new()
        } else {
            format!(" — after {} each month", cuts_phrase(&cuts, code))
        };
        return format!(
            "{} a month on the card, paid the following month{after}.",
            amount(n, code)
        );
    }
    if !cuts.is_empty() {
        return format!("Affordable if {} each month.", cuts_phrase(&cuts, code));
    }
    "Affordable every month.".to_owned()
}

/// What a month could not pay, across all its verdicts.
fn month_short(month: &WishMonth) -> Decimal {
    month.verdicts.iter().map(|v| v.short).sum()
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
    kind: WishKind,
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
struct StartArgs {
    wish_id: String,
    match_category: Option<String>,
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

/// `week`, `2 weeks`, `month`, `year`.
fn period(every: Repeat) -> &'static str {
    match every {
        Repeat::Weekly => "week",
        Repeat::Biweekly => "2 weeks",
        Repeat::Monthly => "month",
        Repeat::Yearly => "year",
    }
}

/// The add form's repeat `<select>` value.
fn parse_repeat(value: &str) -> Repeat {
    match value {
        "weekly" => Repeat::Weekly,
        "biweekly" => Repeat::Biweekly,
        "yearly" => Repeat::Yearly,
        _ => Repeat::Monthly,
    }
}

/// A bill the page can name or change.
#[derive(Clone, Debug)]
struct BillOption {
    id: String,
    name: String,
    amount: Decimal,
}

/// The bills a change can be judged on (the view lists them, with their
/// schedule), labelled `Gym · €30 per month`.
fn changeable_bills(
    bills: &[BillOption],
    changeable: &[ChangeableBill],
    code: &str,
) -> Vec<(BillOption, String)> {
    bills
        .iter()
        .filter_map(|b| {
            let every = changeable.iter().find(|c| c.bill_id == b.id)?.every;
            let label = format!(
                "{} · {} per {}",
                b.name,
                amount(b.amount, code),
                period(every)
            );
            Some((b.clone(), label))
        })
        .collect()
}

/// What the page reads from the dashboard.
#[derive(Clone, Debug, Default)]
struct Context {
    /// Spendable accounts: (id, name).
    accounts: Vec<(String, String)>,
    bills: Vec<BillOption>,
    /// Categories a new bill can match: known, and no bill's yet.
    categories: Vec<String>,
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
        let mut bills: Vec<BillOption> = Vec::new();
        let mut owned: Vec<String> = Vec::new();
        for row in report
            .subscriptions
            .rows
            .iter()
            .chain(report.next_month_subscriptions.iter().flat_map(|s| &s.rows))
        {
            owned.extend(
                row.category_match_categories
                    .iter()
                    .map(|c| c.to_lowercase()),
            );
            if !bills.iter().any(|b| b.id == row.bill_id) {
                bills.push(BillOption {
                    id: row.bill_id.clone(),
                    name: row.name.clone(),
                    amount: row.amount_max,
                });
            }
        }
        let categories = crate::pages::plan::plan_categories(report)
            .into_iter()
            .filter(|c| !owned.contains(&c.to_lowercase()))
            .collect();
        Self {
            accounts,
            bills,
            categories,
        }
    }

    fn bill(&self, bill_id: &str) -> Option<&BillOption> {
        self.bills.iter().find(|b| b.id == bill_id)
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
    /// The add form's kind: `one_off`, `repeats` or `bill`.
    kind: RwSignal<String>,
    /// `weekly`, `biweekly`, `monthly` or `yearly`.
    every: RwSignal<String>,
    ends_on: RwSignal<String>,
    bill: RwSignal<String>,
    /// The wish whose "Start" form is open.
    starting: RwSignal<Option<String>>,
    start_category: RwSignal<String>,
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
            kind: RwSignal::new("one_off".into()),
            every: RwSignal::new("monthly".into()),
            ends_on: RwSignal::new(String::new()),
            bill: RwSignal::new(String::new()),
            starting: RwSignal::new(None),
            start_category: RwSignal::new(String::new()),
        }
    }

    /// Close every row form.
    fn close_forms(self) {
        self.buying.set(None);
        self.dropping.set(None);
        self.starting.set(None);
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
        .filter(|r| r.wish.parent_id.is_none() && r.wish.kind == WishKind::OneOff)
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
    let form = add_form(
        ui,
        &ctx.get_value(),
        &view.changeable_bills,
        parents,
        currency,
    );
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
        .chain(ctx.bills.iter().map(|b| (b.id.clone(), b.name.clone())))
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
            parse_amount(&limit.get_untracked()),
            parse_amount(&used.get_untracked()),
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
            <form class="mt-4 grid items-start gap-3 sm:grid-cols-2 lg:grid-cols-4" on:submit=submit>
                <Field label="Limit" id="card-limit">
                    <input
                        id="card-limit"
                        type="text"
                        inputmode="decimal"
                        pattern=AMOUNT_PATTERN
                        required
                        class=CONTROL
                        prop:value=move || limit.get()
                        on:input=move |e| limit.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Used this month" id="card-used" hint=format!("So far in {month_label}.")>
                    <input
                        id="card-used"
                        type="text"
                        inputmode="decimal"
                        pattern=AMOUNT_PATTERN
                        required
                        class=CONTROL
                        prop:value=move || used.get()
                        on:input=move |e| used.set(event_target_value(&e))
                    />
                </Field>
                <Field label="Card bill" id="card-bill" hint="The bill the card is paid by.">
                    {select("card-bill".into(), bill, bills)}
                </Field>
                <Button submit=true variant=Variant::Secondary class=LABEL_OFFSET disabled=Signal::derive(move || ui.busy.get())>
                    "Save card"
                </Button>
            </form>
            {feedback(ui, "card".into())}
        </Card>
    }
}

/// A decimal amount input bound to `value`.
fn number(id: &'static str, value: RwSignal<String>) -> impl IntoView + use<> {
    view! {
        <input
            id=id
            type="text"
            inputmode="decimal"
            pattern=AMOUNT_PATTERN
            required
            placeholder="0.00"
            class=CONTROL
            prop:value=move || value.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

/// A date input bound to `value`.
fn date_input(id: &'static str, value: RwSignal<String>, required: bool) -> impl IntoView + use<> {
    view! {
        <input
            id=id
            type="date"
            required=required
            class=CONTROL
            prop:value=move || value.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

fn add_form(
    ui: Ui,
    ctx: &Context,
    changeable: &[ChangeableBill],
    parents: Vec<(String, String)>,
    currency: Currency,
) -> impl IntoView + use<> {
    let sources = StoredValue::new(ctx.source_options());
    let parent_options: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Nothing".to_owned()))
            .chain(parents)
            .collect(),
    );
    let bills = changeable_bills(&ctx.bills, changeable, &currency.code);
    let bill_options: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Choose a bill".to_owned()))
            .chain(bills.iter().map(|(b, label)| (b.id.clone(), label.clone())))
            .collect(),
    );
    let bill_names = StoredValue::new(bills.into_iter().map(|(b, _)| b).collect::<Vec<_>>());
    let kinds = vec![
        ("one_off".to_owned(), "One-off".to_owned()),
        ("repeats".to_owned(), "Repeats".to_owned()),
        ("bill".to_owned(), "Change a bill".to_owned()),
    ];
    let repeats = StoredValue::new(vec![
        ("weekly".to_owned(), "Every week".to_owned()),
        ("biweekly".to_owned(), "Every 2 weeks".to_owned()),
        ("monthly".to_owned(), "Every month".to_owned()),
        ("yearly".to_owned(), "Every year".to_owned()),
    ]);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(amount) = parse_amount(&ui.amount.get_untracked()) else {
            ui.set_message("add", "Enter an amount.", true);
            return;
        };
        let ends_on = Some(ui.ends_on.get_untracked()).filter(|e| !e.is_empty());
        let mut name = ui.name.get_untracked().trim().to_owned();
        let (kind, parent) = match ui.kind.get_untracked().as_str() {
            "repeats" => (
                WishKind::Recurring {
                    every: parse_repeat(&ui.every.get_untracked()),
                    ends_on,
                },
                None,
            ),
            "bill" => {
                let bill_id = ui.bill.get_untracked();
                let Some(bill) =
                    bill_names.with_value(|b| b.iter().find(|b| b.id == bill_id).cloned())
                else {
                    ui.set_message("add", "Choose a bill.", true);
                    return;
                };
                if name.is_empty() {
                    name = bill.name;
                }
                (WishKind::BillChange { bill_id, ends_on }, None)
            }
            _ => {
                let parent = ui.parent.get_untracked();
                (WishKind::OneOff, (!parent.is_empty()).then_some(parent))
            }
        };
        let source = if matches!(kind, WishKind::BillChange { .. }) {
            WishSource::Card
        } else {
            parse_source(&ui.source.get_untracked())
        };
        let draft = Draft {
            wish_id: None,
            parent_id: parent,
            name,
            amount,
            currency: currency.clone(),
            due_on: ui.due_on.get_untracked(),
            source,
            kind,
        };
        ui.write(
            "add".into(),
            "save_wish",
            DraftArgs { draft },
            "Wish added.",
            move || {
                for field in [
                    ui.name, ui.amount, ui.due_on, ui.parent, ui.ends_on, ui.bill,
                ] {
                    field.set(String::new());
                }
            },
        );
    };
    // The fields that differ by kind.
    let fields = move || {
        match ui.kind.get().as_str() {
        "repeats" => view! {
            <Field label="Each time" id="wish-amount">{number("wish-amount", ui.amount)}</Field>
            <Field label="Repeats" id="wish-every">
                {select("wish-every".into(), ui.every, repeats.get_value())}
            </Field>
            <Field label="First date" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Ends" id="wish-ends" hint="Optional.">{date_input("wish-ends", ui.ends_on, false)}</Field>
            <Field label="Pay with" id="wish-source">
                {select("wish-source".into(), ui.source, sources.get_value())}
            </Field>
        }
        .into_any(),
        "bill" => view! {
            <Field label="Bill" id="wish-bill">
                {select("wish-bill".into(), ui.bill, bill_options.get_value())}
            </Field>
            <Field label="New amount" id="wish-amount" hint="Per payment; 0 cancels the bill.">
                {number("wish-amount", ui.amount)}
            </Field>
            <Field label="From" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Ends" id="wish-ends" hint="Optional.">{date_input("wish-ends", ui.ends_on, false)}</Field>
        }
        .into_any(),
        _ => view! {
            <Field label="Amount" id="wish-amount" hint="For a wish with parts, the most it may cost.">
                {number("wish-amount", ui.amount)}
            </Field>
            <Field label="Date" id="wish-date">{date_input("wish-date", ui.due_on, true)}</Field>
            <Field label="Pay with" id="wish-source">
                {select("wish-source".into(), ui.source, sources.get_value())}
            </Field>
            <Field label="Part of" id="wish-parent">
                {select("wish-parent".into(), ui.parent, parent_options.get_value())}
            </Field>
        }
        .into_any(),
    }
    };
    view! {
        <form class="mt-4 grid items-start gap-3 sm:grid-cols-2 lg:grid-cols-3" on:submit=submit>
            <Field label="Kind" id="wish-kind">{select("wish-kind".into(), ui.kind, kinds)}</Field>
            <Field label="Name" id="wish-name">
                <input
                    id="wish-name"
                    required=move || ui.kind.get() != "bill"
                    maxlength="128"
                    placeholder=move || if ui.kind.get() == "bill" { "The bill's name" } else { "e.g. Winter jacket" }
                    class=CONTROL
                    prop:value=move || ui.name.get()
                    on:input=move |e| ui.name.set(event_target_value(&e))
                />
            </Field>
            {fields}
            <Button submit=true variant=Variant::Secondary class="sm:col-start-1" disabled=Signal::derive(move || ui.busy.get())>
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
    let tone = if row.not_judged.is_some() {
        "text-fg-muted"
    } else if row.short > Decimal::ZERO {
        "text-negative-fg"
    } else if row.tradeoffs {
        "text-warning-fg"
    } else {
        "text-positive-fg"
    };
    let until = |ends_on: &Option<String>| {
        ends_on
            .as_deref()
            .map(|e| format!(" · until {}", format::date(e)))
            .unwrap_or_default()
    };
    let detail = match &wish.kind {
        WishKind::OneOff => format!(
            "{} · by {} · {}",
            amount(wish.amount, &code),
            format::date(&wish.due_on),
            ctx.with_value(|c| c.source_label(&wish.source))
        ),
        WishKind::Recurring { every, ends_on } => format!(
            "{} per {} · from {}{} · {}",
            amount(wish.amount, &code),
            period(*every),
            format::date(&wish.due_on),
            until(ends_on),
            ctx.with_value(|c| c.source_label(&wish.source))
        ),
        WishKind::BillChange { bill_id, ends_on } => {
            let old = ctx.with_value(|c| c.bill(bill_id).map(|b| amount(b.amount, &code)));
            format!(
                "{}→ {} · from {}{}",
                old.map(|o| format!("{o} ")).unwrap_or_default(),
                amount(wish.amount, &code),
                format::date(&wish.due_on),
                until(ends_on)
            )
        }
    };
    let one_off = wish.kind == WishKind::OneOff;
    let start_text = match &wish.kind {
        WishKind::BillChange { bill_id, ends_on } => {
            let bill = ctx.with_value(|c| c.bill(bill_id).map(|b| (b.name.clone(), b.amount)));
            start_confirm(bill, wish.amount, ends_on.as_deref(), &code)
        }
        _ => String::new(),
    };
    let recurring = matches!(wish.kind, WishKind::Recurring { .. });
    let target = format!("wish:{}", wish.wish_id);
    let saved = RwSignal::new(wish.saved.to_string());
    let id = StoredValue::new(wish.wish_id.clone());
    let save_saved = {
        let target = target.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            let Ok(value) = parse_amount(&saved.get_untracked()) else {
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
            ui.close_forms();
            ui.buy_amount.set(wish.amount.to_string());
            ui.buy_on.set(today());
            ui.buy_source.set(source_value(&wish.source));
            ui.buying.set(Some(wish.wish_id.clone()));
        }
    };
    let open_start = move |_| {
        ui.close_forms();
        ui.start_category.set(String::new());
        ui.starting.set(Some(id.get_value()));
    };
    let buying = move || ui.buying.get().as_deref() == Some(id.get_value().as_str());
    let starting = move || ui.starting.get().as_deref() == Some(id.get_value().as_str());
    let start_target = target.clone();
    let start = move || {
        let match_category = if recurring {
            let category = ui.start_category.get_untracked();
            if category.is_empty() {
                ui.set_message(&start_target, "Choose the bill's category.", true);
                return;
            }
            Some(category)
        } else {
            None
        };
        ui.write(
            start_target.clone(),
            "start_wish",
            StartArgs {
                wish_id: id.get_value(),
                match_category,
            },
            "Started.",
            move || ui.starting.set(None),
        );
    };
    let start = StoredValue::new(start);
    let categories: StoredValue<Vec<(String, String)>> = StoredValue::new(
        std::iter::once((String::new(), "Choose a category".to_owned()))
            .chain(ctx.with_value(|c| {
                c.categories
                    .iter()
                    .map(|x| (x.clone(), x.clone()))
                    .collect::<Vec<_>>()
            }))
            .collect(),
    );
    let start_category_id = StoredValue::new(format!("start-category-{}", wish.wish_id));
    let dropping = move || ui.dropping.get().as_deref() == Some(id.get_value().as_str());
    let sources = ctx.with_value(Context::source_options);
    let buy_target = target.clone();
    let buy_submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = parse_amount(&ui.buy_amount.get_untracked()) else {
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
                {one_off.then(move || view! {
                <form class="flex items-end gap-2" on:submit=save_saved>
                    <div class="grid gap-1">
                        <label for=saved_id.clone() class="text-xs text-fg-muted">"Saved so far"</label>
                        <input
                            id=saved_id
                            type="text"
                            inputmode="decimal"
                            pattern=AMOUNT_PATTERN
                            class=cn([CONTROL, "h-8 w-28"])
                            prop:value=move || saved.get()
                            on:input=move |e| saved.set(event_target_value(&e))
                        />
                    </div>
                    <Button submit=true size=Size::Sm variant=Variant::Secondary disabled=Signal::derive(move || ui.busy.get())>
                        "Save"
                    </Button>
                </form>
                })}
                {if one_off {
                    view! {
                        <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_buy)>
                            "Bought"
                        </Button>
                    }
                    .into_any()
                } else {
                    view! {
                        <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_start)>
                            "Start"
                        </Button>
                    }
                    .into_any()
                }}
                <Button
                    size=Size::Sm
                    variant=Variant::Ghost
                    on_click=Callback::new(move |_| {
                        ui.close_forms();
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
                            type="text"
                            inputmode="decimal"
                            pattern=AMOUNT_PATTERN
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
                            max=today()
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
            <Show when=move || starting() && recurring>
                <form
                    class="grid items-end gap-2 sm:grid-cols-3"
                    on:submit=move |ev: SubmitEvent| {
                        ev.prevent_default();
                        start.with_value(|f| f());
                    }
                >
                    <Field
                        label="Bill category"
                        id=start_category_id.get_value()
                        hint="Payments in this category mark the new bill paid."
                    >
                        {select(start_category_id.get_value(), ui.start_category, categories.get_value())}
                    </Field>
                    <div class=cn(["flex gap-2", LABEL_OFFSET])>
                        <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Start"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| ui.starting.set(None))>"Cancel"</Button>
                    </div>
                </form>
            </Show>
            <Show when=move || starting() && !recurring>
                <div class="flex flex-wrap items-center gap-2 text-sm">
                    <span>{start_text.clone()}</span>
                    <Button
                        size=Size::Sm
                        disabled=Signal::derive(move || ui.busy.get())
                        on_click=Callback::new(move |_| start.with_value(|f| f()))
                    >
                        "Start"
                    </Button>
                    <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| ui.starting.set(None))>"Keep"</Button>
                </div>
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

/// What starting a bill change asks; with an end date it warns that the
/// old amount does not come back on its own.
fn start_confirm(
    bill: Option<(String, Decimal)>,
    new_amount: Decimal,
    ends_on: Option<&str>,
    code: &str,
) -> String {
    let (name, old) = bill.map_or((None, None), |(n, a)| (Some(n), Some(a)));
    let name = name.unwrap_or_else(|| "the bill".to_owned());
    let ask = if new_amount.is_zero() {
        format!("Archive {name} now? Its payments stop counting from today.")
    } else {
        format!(
            "Change {name} to {} now? The bill keeps its schedule.",
            amount(new_amount, code)
        )
    };
    match ends_on {
        None => ask,
        Some(end) => {
            let back = old.map_or_else(
                || "the old amount".to_owned(),
                |o| format!("the old amount ({})", amount(o, code)),
            );
            format!(
                "{ask} After {}, {back} won't come back on its own: change the bill back then.",
                format::date(end)
            )
        }
    }
}

/// A bought, started or dropped wish in one muted line, with its parts
/// below it (they are no longer judged once the parent is settled).
fn done_row(row: &WishRow, ctx: &Context) -> impl IntoView + use<> {
    let parts = row
        .children
        .iter()
        .map(|child| view! { <p class="py-1 pl-4 sm:pl-6">{done_text(&child.wish, ctx)}</p> })
        .collect_view();
    view! {
        <p class="py-1">{done_text(&row.wish, ctx)}</p>
        {parts}
    }
}

fn done_text(wish: &Wish, ctx: &Context) -> String {
    let code = &wish.currency.code;
    match (&wish.status, &wish.bought) {
        (WishStatus::Bought, Some(p)) => format!(
            "{} · bought {} on {} · {}",
            wish.name,
            amount(p.amount, code),
            format::date(&p.on),
            ctx.source_label(&p.source)
        ),
        (WishStatus::Bought, None) => format!("{} · bought", wish.name),
        (WishStatus::Started, _) => format!("{} · started", wish.name),
        (WishStatus::Planned, _) => format!(
            "{} · not bought · {} (no longer judged)",
            wish.name,
            amount(wish.amount, code)
        ),
        (WishStatus::Dropped, _) => {
            format!("{} · dropped · {}", wish.name, amount(wish.amount, code))
        }
    }
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
    let short = month_short(month);
    let short = (short > Decimal::ZERO).then(|| {
        view! {
            <p role="status" class="text-sm font-medium text-negative-fg">
                {format!("Short by {}", amount(short, code))}
            </p>
        }
    });
    view! {
        <Card tone=CardTone::Muted class="grid gap-3 p-4 sm:p-5">
            <h3 class="text-sm font-semibold">{title}</h3>
            <div class="grid grid-cols-2 gap-3">
                <Stat label="Free before">{before}</Stat>
                <Stat label="Free after">{after}</Stat>
                <Stat label="Card charge in">{charge}</Stat>
                <Stat label="Card used">{used}</Stat>
            </div>
            {short}
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                saved_month: None,
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
            not_judged: None,
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
            set_aside: false,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
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

    fn month_at(at: &str, verdicts: Vec<Verdict>) -> WishMonth {
        WishMonth {
            month: at.into(),
            ..month(Decimal::ZERO, Decimal::ZERO, verdicts)
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
            on_card: d(100, 0),
            ..verdict("w", d(400, 0))
        };
        let later = Verdict {
            on_card: d(30, 0),
            ..verdict("w", d(30, 0))
        };
        let months = [
            month(Decimal::ZERO, Decimal::ZERO, vec![v]),
            month_at("2026-11-01", vec![later]),
        ];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable with €130 on the card — November starts €130 lighter."
        );
    }

    #[test]
    fn card_use_also_names_the_cuts() {
        let v = Verdict {
            cuts: vec![
                cut("g1", "Going out", 120, 0),
                cut("g2", "Groceries", 450, 300),
            ],
            on_card: d(130, 0),
            ..verdict("w", d(400, 0))
        };
        let months = [
            month_at("2026-09-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![v]),
        ];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable with €130 on the card — December starts €130 lighter — after going out drops to €0 and groceries to €300."
        );
    }

    fn repeating(id: &str, every: Repeat) -> WishRow {
        let mut r = row(id);
        r.wish.kind = WishKind::Recurring {
            every,
            ends_on: None,
        };
        r
    }

    fn paid(amount: i64) -> Verdict {
        Verdict {
            from_free: d(amount, 0),
            ..verdict("w", d(amount, 0))
        }
    }

    #[test]
    fn recurring_that_fits_is_affordable_every_month() {
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![paid(60)]),
            month_at("2026-12-01", vec![paid(60)]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "Affordable every month."
        );
    }

    #[test]
    fn recurring_cuts_hold_each_month() {
        let v = Verdict {
            cuts: vec![cut("g1", "Going out", 120, 60)],
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![v]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "Affordable if going out drops to €60 each month."
        );
    }

    #[test]
    fn recurring_on_the_card_is_paid_the_following_month() {
        let card = |n| Verdict {
            on_card: d(n, 0),
            ..verdict("w", d(n, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![card(40)]),
            month_at("2026-12-01", vec![card(60)]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "€40 a month on the card, paid the following month."
        );
        let with_cuts = Verdict {
            cuts: vec![cut("g1", "Going out", 120, 0)],
            ..card(40)
        };
        let months = [month_at("2026-10-01", vec![with_cuts])];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "€40 a month on the card, paid the following month — after going out drops to €0 each month."
        );
    }

    #[test]
    fn recurring_fits_until_a_short_month() {
        let short = Verdict {
            short: d(25, 0),
            ..verdict("w", d(60, 0))
        };
        let later = Verdict {
            short: d(60, 0),
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![paid(60)]),
            month_at("2026-11-01", vec![paid(60)]),
            month_at("2026-12-01", vec![short]),
            month_at("2027-01-01", vec![later]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "Fits until November. From December, short by €25."
        );
    }

    #[test]
    fn recurring_short_from_the_start_is_not_affordable() {
        let short = Verdict {
            short: d(15, 0),
            on_card: d(45, 0),
            ..verdict("w", d(60, 0))
        };
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![short]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Weekly), &months),
            "Not affordable — short by €15 from November."
        );
    }

    #[test]
    fn cheaper_bill_frees_money() {
        let mut r = row("w");
        r.wish.kind = WishKind::BillChange {
            bill_id: "7".into(),
            ends_on: None,
        };
        let freed = Verdict {
            from_free: d(-15, 0),
            ..verdict("w", d(-15, 0))
        };
        let months = [
            month_at("2026-10-01", vec![verdict("other", d(5, 0))]),
            month_at("2026-11-01", vec![freed.clone()]),
            month_at("2026-12-01", vec![freed]),
        ];
        assert_eq!(
            verdict_line(&r, &months),
            "Frees €15 a month from November."
        );
    }

    #[test]
    fn freed_money_is_not_a_savings_drop() {
        let freed = Verdict {
            from_free: d(-15, 0),
            ..verdict("w", d(-15, 0))
        };
        let months = [month(d(100, 0), d(115, 0), vec![freed])];
        assert_eq!(verdict_line(&row("w"), &months), "Affordable.");
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
    fn card_charge_next_month_cannot_pay_names_the_short() {
        let v = Verdict {
            on_card: d(500, 0),
            next_short: d(500, 0),
            ..verdict("w", d(500, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Not affordable — €500 on the card, so November starts €500 short."
        );
    }

    #[test]
    fn card_charge_short_also_names_its_cuts() {
        let v = Verdict {
            on_card: d(500, 0),
            next_short: d(380, 0),
            next_cuts: vec![cut("g1", "Going out", 120, 0)],
            ..verdict("w", d(500, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Not affordable — €500 on the card, so November starts €380 short even after going out drops to €0."
        );
    }

    #[test]
    fn card_charge_paid_by_cuts_names_them() {
        let v = Verdict {
            on_card: d(100, 0),
            next_cuts: vec![cut("g1", "Going out", 120, 20)],
            ..verdict("w", d(100, 0))
        };
        let months = [month(Decimal::ZERO, Decimal::ZERO, vec![v])];
        assert_eq!(
            verdict_line(&row("w"), &months),
            "Affordable with €100 on the card — November starts €100 lighter, so going out drops to €20 in November."
        );
    }

    #[test]
    fn recurring_card_charge_short_shows_the_month_after() {
        let card = |n, next_short| Verdict {
            on_card: d(n, 0),
            next_short: d(next_short, 0),
            ..verdict("w", d(n, 0))
        };
        let months = [
            month_at("2026-10-01", vec![card(60, 0)]),
            month_at("2026-11-01", vec![card(60, 20)]),
            month_at("2026-12-01", vec![card(60, 60)]),
        ];
        assert_eq!(
            verdict_line(&repeating("w", Repeat::Monthly), &months),
            "Fits until November. From December, short by €20."
        );
    }

    #[test]
    fn month_short_sums_every_verdict() {
        let charge = Verdict {
            wish_id: None,
            short: d(30, 0),
            ..verdict("x", d(30, 0))
        };
        let tv = Verdict {
            short: d(12, 0),
            ..verdict("w", d(12, 0))
        };
        assert_eq!(
            month_short(&month(Decimal::ZERO, Decimal::ZERO, vec![charge, tv])),
            d(42, 0)
        );
    }

    #[test]
    fn bill_change_with_an_end_warns_on_start() {
        let gym = Some(("Gym".to_owned(), d(30, 0)));
        assert_eq!(
            start_confirm(gym.clone(), d(45, 0), None, "EUR"),
            "Change Gym to €45 now? The bill keeps its schedule."
        );
        let text = start_confirm(gym, d(45, 0), Some("2027-03-01"), "EUR");
        assert!(text.starts_with("Change Gym to €45 now? The bill keeps its schedule. After "));
        assert!(text.ends_with(
            ", the old amount (€30) won't come back on its own: change the bill back then."
        ));
    }

    #[test]
    fn unjudged_wish_says_why() {
        let mut r = row("w");
        r.not_judged = Some("too far ahead".into());
        assert_eq!(verdict_line(&r, &[]), "Not judged yet: too far ahead.");
    }

    #[test]
    fn bill_picker_lists_judgeable_bills_with_their_schedule() {
        let bill = |id: &str, name: &str| BillOption {
            id: id.into(),
            name: name.into(),
            amount: d(80, 0),
        };
        let bills = [bill("9", "Therapy"), bill("8", "Insurance")];
        let changeable = [ChangeableBill {
            bill_id: "9".into(),
            every: Repeat::Biweekly,
        }];
        let options = changeable_bills(&bills, &changeable, "EUR");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].1, "Therapy · €80 per 2 weeks");
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
