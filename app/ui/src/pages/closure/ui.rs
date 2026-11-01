//! Shared classes, controls and helpers of the month-closure views
//! (personal-site `components/month-closure/ui.tsx` and `api.ts`).

use std::cell::Cell;

use finance_types::report::Currency;
use finance_types::report::closure::{
    AccountRoleDraft, ClosureAction, ClosureView, UnclassifiedAccounts,
};
use leptos::prelude::*;
use rust_decimal::Decimal;
use serde::Serialize;
use wasm_bindgen::JsValue;

use crate::api::{NoArgs, invoke};
use crate::ds::{CONTROL, Icon, Lucide, card_class, cn};
use crate::format;

pub const FOOTNOTE: &str = "text-sm leading-relaxed text-fg-muted";
pub const SMALL: &str = "block text-xs text-fg-muted";
pub const WARN: &str = "text-sm text-negative-fg";
pub const ACTIONS: &str = "flex flex-wrap items-center gap-2";
pub const CALLOUT: &str =
    "grid gap-2 rounded-lg border border-border bg-card-muted px-4 py-3 text-sm";
pub const LIST: &str = "grid divide-y divide-line";
pub const SUBHEADING: &str = "mt-2 text-sm font-semibold";
pub const ERROR: &str =
    "rounded-lg border border-negative/30 bg-negative/10 px-4 py-3 text-sm text-negative-fg";
/// Inline forms (new pot, account type, planned purchase): fields wrap,
/// buttons sit on the field baseline.
pub const INLINE_FORM: &str = "flex flex-wrap items-end gap-3";
/// A field inside an inline form: grows from 11rem and wraps below that.
pub const GROW: &str = "min-w-0 flex-1 basis-44";
pub const LINK: &str = "font-medium underline underline-offset-2 hover:text-fg";

/// The revision conflict the backend reports; the screen reloads the month.
pub const CONFLICT: &str = "month closure changed; refresh";

/// One panel of the closure flow: a card with a heading, an optional
/// explanation and its content.
#[component]
pub fn StepCard(
    #[prop(into)] title: String,
    #[prop(default = None)] description: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=cn([card_class(Default::default()), "grid min-w-0 gap-4"])>
            <div>
                <h3 class="text-lg font-semibold leading-tight">{title}</h3>
                {description.map(|d| view! { <p class="mt-1 text-sm leading-relaxed text-fg-muted">{d}</p> })}
            </div>
            {children()}
        </div>
    }
}

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

/// A unique element id for a label/control pair.
pub fn field_id() -> String {
    NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        format!("closure-field-{}", n.get())
    })
}

/// The site's `Field`: a label over its control.
#[component]
pub fn FieldBox(
    label: AnyView,
    #[prop(into)] id: String,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=cn(["grid gap-1.5", class.as_str()])>
            <label for=id class="text-sm font-medium text-fg">{label}</label>
            {children()}
        </div>
    }
}

/// The site's `Select` with a change handler; `options` are
/// (value, label, disabled) triples.
#[component]
pub fn Choice(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] options: Signal<Vec<(String, String, bool)>>,
    on_change: Callback<String>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] label: Option<String>,
    #[prop(optional, into)] title: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="relative w-full">
            <select
                id=id
                aria-label=label
                title=title
                class=cn([CONTROL, "appearance-none pr-9"])
                disabled=move || disabled.get()
                on:change=move |e| {
                    let _ = on_change.try_run(event_target_value(&e));
                }
                prop:value=move || value.get()
            >
                {move || {
                    let current = value.get();
                    options
                        .get()
                        .into_iter()
                        .map(|(v, l, off)| {
                            let selected = v == current;
                            view! { <option value=v selected=selected disabled=off>{l}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                <Lucide icon=Icon::ChevronDown size=16 />
            </span>
        </div>
    }
}

/// `(value, label)` pairs as enabled options.
pub fn options<I, A, B>(items: I) -> Vec<(String, String, bool)>
where
    I: IntoIterator<Item = (A, B)>,
    A: Into<String>,
    B: Into<String>,
{
    items
        .into_iter()
        .map(|(v, l)| (v.into(), l.into(), false))
        .collect()
}

/// The site's `Input`: a plain control whose value comes from `value`.
#[component]
pub fn TextBox(
    #[prop(into)] value: Signal<String>,
    on_input: Callback<String>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] kind: Option<String>,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(optional, into)] inputmode: Option<String>,
    #[prop(optional, into)] min: Option<String>,
    #[prop(optional)] maxlength: Option<usize>,
    #[prop(optional, into)] invalid: Signal<bool>,
    #[prop(optional, into)] on_blur: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <input
            id=id
            type=kind.unwrap_or_else(|| "text".into())
            class=CONTROL
            placeholder=placeholder
            inputmode=inputmode
            min=min
            maxlength=maxlength
            aria-invalid=move || invalid.get().to_string()
            prop:value=move || value.get()
            on:input=move |e| {
                let _ = on_input.try_run(event_target_value(&e));
            }
            on:blur=move |_| {
                if let Some(f) = on_blur {
                    let _ = f.try_run(());
                }
            }
        />
    }
}

/// An in-app link styled like the site's `<a className={linkClass}>`.
#[component]
pub fn TextLink(on_click: Callback<()>, children: Children) -> impl IntoView {
    view! {
        <button type="button" class=LINK on:click=move |_| {
            let _ = on_click.try_run(());
        }>
            {children()}
        </button>
    }
}

// ---------------------------------------------------------------- months

fn iso(year: i32, month_index: i32) -> String {
    let index = year * 12 + month_index;
    format!(
        "{:04}-{:02}-01",
        index.div_euclid(12),
        index.rem_euclid(12) + 1
    )
}

fn today() -> (i32, i32, u32) {
    let now = js_sys::Date::new_0();
    (
        now.get_full_year() as i32,
        now.get_month() as i32,
        now.get_date(),
    )
}

/// The month to close: the previous month until the 24th, the current month
/// from the 25th.
pub fn month_to_close() -> String {
    let (year, month, day) = today();
    iso(year, month - i32::from(day <= 24))
}

/// The month to close and the two months before it, newest first.
pub fn recent_months() -> Vec<String> {
    let target = month_to_close();
    (0..3).map(|back| shift(&target, -back)).collect()
}

/// `YYYY-MM-01` shifted by `delta` months.
pub fn shift(month: &str, delta: i32) -> String {
    format!(
        "{}-01",
        format::add_months(&month[..7.min(month.len())], delta)
    )
}

pub fn previous_month(month: &str) -> String {
    shift(month, -1)
}

pub fn next_month(month: &str) -> String {
    shift(month, 1)
}

/// "October 2026", or "October" without the year.
pub fn month_label(month: &str, with_year: bool) -> String {
    let long = format::month_long(month);
    if with_year {
        long
    } else {
        long.split(' ').next().unwrap_or_default().to_owned()
    }
}

/// WebKit parses only millisecond fractions; Python wrote microseconds.
fn js_date(value: &str) -> Option<js_sys::Date> {
    let mut text = String::with_capacity(value.len());
    let mut fraction = None::<usize>;
    for c in value.chars() {
        match fraction {
            Some(n) if c.is_ascii_digit() => {
                if n < 3 {
                    text.push(c);
                }
                fraction = Some(n + 1);
            }
            _ => {
                fraction = (c == '.').then_some(0);
                text.push(c);
            }
        }
    }
    let date = js_sys::Date::new(&JsValue::from_str(&text));
    (!date.get_time().is_nan()).then_some(date)
}

/// `26 Oct, 21:07` (en-GB), or "not recorded".
pub fn date_time(value: Option<&str>) -> String {
    let Some(date) = value.and_then(js_date) else {
        return "not recorded".into();
    };
    let options = js_sys::Object::new();
    for (key, value) in [
        ("day", "numeric"),
        ("month", "short"),
        ("hour", "2-digit"),
        ("minute", "2-digit"),
    ] {
        let _ = js_sys::Reflect::set(&options, &key.into(), &value.into());
    }
    date.to_locale_string("en-GB", &options).into()
}

// ---------------------------------------------------------------- money

/// `formatUnitMoney`.
pub fn money(value: Decimal, unit: &Currency) -> String {
    crate::ds::money(value, unit)
}

/// An amount with exactly the currency's minor-unit decimals
/// ("1750.440000" becomes "1750.44"). Text that is not a number, or has more
/// significant decimals than the currency allows, is kept so validation can
/// flag it.
pub fn minor_amount(value: &str, minor_unit: u8) -> String {
    let text = value.trim();
    match parse_amount(text) {
        Some(amount) if amount.normalize().scale() <= u32::from(minor_unit) => {
            fixed(amount, minor_unit)
        }
        _ => value.to_owned(),
    }
}

/// `/^-?\d+(\.\d+)?$/` as a decimal.
pub fn parse_amount(text: &str) -> Option<Decimal> {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (whole, frac) = match body.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (body, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    text.parse().ok()
}

/// `Decimal.toFixed(unit)`.
pub fn fixed(value: Decimal, unit: u8) -> String {
    let rounded = value.round_dp_with_strategy(
        u32::from(unit),
        rust_decimal::RoundingStrategy::MidpointAwayFromZero,
    );
    format!("{:.*}", usize::from(unit), rounded)
}

/// Significant decimal places, like `Decimal.decimalPlaces()`.
pub fn decimal_places(value: Decimal) -> u32 {
    value.normalize().scale()
}

/// A random 32-character line id (`crypto.randomUUID().slice(0, 32)`).
pub fn new_id() -> String {
    (0..32)
        .map(|_| {
            let n = (js_sys::Math::random() * 16.0) as u32;
            char::from_digit(n.min(15), 16).unwrap_or('0')
        })
        .collect()
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
struct MonthArgs {
    month: String,
}

#[derive(Serialize)]
struct ActionArgs {
    action: ClosureAction,
}

#[derive(Serialize)]
struct RoleArgs {
    role: AccountRoleDraft,
}

pub async fn load_closure(month: &str) -> Result<ClosureView, String> {
    invoke(
        "closure",
        &MonthArgs {
            month: month.to_owned(),
        },
    )
    .await
}

pub async fn load_unclassified() -> Result<UnclassifiedAccounts, String> {
    invoke("unclassified_accounts", &NoArgs {}).await
}

pub async fn post_closure(action: ClosureAction) -> Result<ClosureView, String> {
    invoke("closure_action", &ActionArgs { action }).await
}

/// `via` `None` keeps the saved route, `Some(None)` clears it.
pub async fn post_role(
    account_id: String,
    role: &str,
    goal_id: Option<String>,
    via: Option<Option<String>>,
) -> Result<(), String> {
    invoke(
        "save_account_role",
        &RoleArgs {
            role: AccountRoleDraft {
                account_id,
                role: role.to_owned(),
                goal_id,
                via_account_id: via,
            },
        },
    )
    .await
}
