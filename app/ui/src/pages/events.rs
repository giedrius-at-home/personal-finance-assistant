//! The Events section: wishes with dates. An event owns the day-to-day
//! spending of its days, is judged on what that costs above the usual, and
//! is paced while it runs. Each event is the wish row it also is (parts,
//! saved money, must), with its own numbers, payments and dates around it.

use finance_types::gateway::{EventDetails, EventPick, WishKind, WishSource};
use finance_types::report::Currency;
use finance_types::report::events::{EventPhase, EventRow, EventsView, Pace};
use finance_types::report::review::SearchRow;
use finance_types::report::wishes::WishMonth;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::{NoArgs, load};
use crate::ds::{
    BOARD_CARD, Badge, BoardColumn, Button, CONTROL, Card, Checkbox, Eyebrow, Facts, FactsRow,
    Field, Icon, Lucide, Popup, Size, Skeleton, Tone, Variant, cn,
};
use crate::format;
use crate::pages::Problem;
use crate::pages::payment_pick::{Ask, PaymentPicker};
use crate::pages::wishes::{
    AMOUNT_PATTERN, Context, Draft, DraftArgs, Fact, Ui, WishIdArgs, amount, feedback,
    parse_amount, today, wish_row,
};

#[derive(Serialize)]
struct PickArgs {
    wish_id: String,
    transaction_id: String,
    pick: EventPick,
}

/// How an event's amount counts: `total` (its parts come out of it) or
/// `extra` (on top of its parts).
const MODES: [(&str, &str); 2] = [
    ("total", "In total, parts included"),
    ("extra", "Extra, on top of the parts"),
];

fn mode_select(id: String, value: RwSignal<String>) -> impl IntoView {
    view! {
        <select
            id=id
            class=CONTROL
            prop:value=move || value.get()
            on:change=move |e| value.set(event_target_value(&e))
        >
            {MODES
                .into_iter()
                .map(|(v, label)| {
                    let selected = v == value.get_untracked();
                    view! { <option value=v selected=selected>{label}</option> }
                })
                .collect_view()}
        </select>
    }
}

fn on_top(e: &EventRow) -> bool {
    e.row.wish.event.as_ref().is_some_and(|d| d.on_top)
}

/// Everything the event is planned to cost: its amount, and its parts too
/// when the amount is on top of them.
pub fn planned_total(e: &EventRow) -> Decimal {
    let parts: Decimal = if on_top(e) {
        e.row
            .children
            .iter()
            .map(|c| match c.wish.status {
                finance_types::gateway::WishStatus::Planned => c.wish.amount,
                finance_types::gateway::WishStatus::Bought => {
                    c.wish.bought.as_ref().map_or(c.wish.amount, |p| p.amount)
                }
                _ => Decimal::ZERO,
            })
            .sum()
    } else {
        Decimal::ZERO
    };
    e.row.wish.amount + parts
}

/// The event's own numbers, one value per fact, only those that apply.
pub fn event_facts(e: &EventRow, code: &str) -> Vec<Fact> {
    let m = |value: Decimal| amount(value, code);
    let wish = &e.row.wish;
    let mut facts = vec![("Planned".to_owned(), m(planned_total(e)))];
    let mut push = |label: &str, value: String| facts.push((label.to_owned(), value));
    if on_top(e) {
        push("Of it, for the days", m(wish.amount));
    }
    let parts: Decimal = e
        .row
        .children
        .iter()
        .filter(|c| c.wish.status == finance_types::gateway::WishStatus::Planned)
        .map(|c| c.wish.amount)
        .sum();
    if parts > Decimal::ZERO {
        push("Parts planned", m(parts));
    }
    if e.paid_ahead > Decimal::ZERO {
        push("Paid ahead", m(e.paid_ahead));
    }
    push("Usual those days", m(e.usual));
    push("Extra to find", m(e.extra));
    if e.phase == EventPhase::Upcoming {
        if let Some(fits) = e.fits_up_to {
            push("Fits without trade-offs", m(fits));
        }
    } else {
        push("Spent", m(e.spent));
    }
    if e.phase == EventPhase::Running {
        push("Left", m(e.left));
        push("Days left", e.days_left.to_string());
        if e.left > Decimal::ZERO {
            push("A day, to stay inside", m(e.per_day));
        }
    }
    facts
}

/// The chip that says how a running or finished event stands; an upcoming
/// one is judged by its wish row's own chip.
pub fn standing(e: &EventRow) -> Option<(Tone, &'static str)> {
    match (e.phase, e.pace) {
        (EventPhase::Upcoming, _) => None,
        (EventPhase::Past, _) => Some((Tone::Outline, "Done")),
        (EventPhase::Running, Pace::OnTrack) => Some((Tone::Positive, "On track")),
        (EventPhase::Running, Pace::Fast) => Some((Tone::Warning, "Spending fast")),
        (EventPhase::Running, Pace::Over) => Some((Tone::Warning, "Over")),
    }
}

/// Form state of the screen, beside the wish rows' own.
#[derive(Clone, Copy)]
struct Own {
    name: RwSignal<String>,
    amount: RwSignal<String>,
    starts: RwSignal<String>,
    ends: RwSignal<String>,
    must: RwSignal<bool>,
    /// The event whose form is open, and which: `part` or `dates`.
    open: RwSignal<Option<(String, &'static str)>>,
    part_name: RwSignal<String>,
    part_amount: RwSignal<String>,
    part_on: RwSignal<String>,
    new_starts: RwSignal<String>,
    new_ends: RwSignal<String>,
    new_amount: RwSignal<String>,
    /// `total` or `extra`: the add form's, and the change form's.
    mode: RwSignal<String>,
    new_mode: RwSignal<String>,
}

impl Own {
    fn new() -> Self {
        let text = || RwSignal::new(String::new());
        Self {
            name: text(),
            amount: text(),
            starts: text(),
            ends: text(),
            must: RwSignal::new(false),
            open: RwSignal::new(None),
            part_name: text(),
            part_amount: text(),
            part_on: text(),
            new_starts: text(),
            new_ends: text(),
            new_amount: text(),
            mode: RwSignal::new("total".into()),
            new_mode: RwSignal::new("total".into()),
        }
    }

    fn is_open(self, id: &str, form: &'static str) -> bool {
        self.open
            .with(|o| o.as_ref().is_some_and(|(i, f)| i == id && *f == form))
    }

    fn toggle(self, id: &str, form: &'static str) {
        let open = self.is_open(id, form);
        self.open.set((!open).then(|| (id.to_owned(), form)));
    }
}

fn text_input(id: String, value: RwSignal<String>, kind: &'static str) -> impl IntoView {
    let (inputmode, pattern) = if kind == "amount" {
        (Some("decimal"), Some(AMOUNT_PATTERN))
    } else {
        (None, None)
    };
    view! {
        <input
            id=id
            type=if kind == "date" { "date" } else { "text" }
            inputmode=inputmode
            pattern=pattern
            required
            maxlength="128"
            class=CONTROL
            prop:value=move || value.get()
            on:input=move |e| value.set(event_target_value(&e))
        />
    }
}

#[component]
pub fn Events() -> impl IntoView {
    let ui = Ui::new();
    let own = Own::new();
    let revision = ui.app.revision;
    let view_data = load::<EventsView, _>("events", move || {
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
            Some(Ok(view)) => page(ui, own, view, ctx).into_any(),
        })
    }
}

fn page(ui: Ui, own: Own, view: &EventsView, ctx: Context) -> impl IntoView + use<> {
    let months = StoredValue::new(view.months.clone());
    let ctx = StoredValue::new(ctx);
    let currency = view.currency.clone();
    let form = add_form(ui, own, currency.clone());
    // The board: every event a box under its phase; the one picked opens
    // with its numbers, parts and payments under the board.
    let board = [
        ("Upcoming", Tone::Accent, EventPhase::Upcoming),
        ("Running", Tone::Positive, EventPhase::Running),
        ("Past", Tone::Neutral, EventPhase::Past),
    ]
    .into_iter()
    .map(|(title, tone, phase)| {
        let boxes: Vec<_> = view
            .events
            .iter()
            .filter(|e| e.phase == phase)
            .map(|e| event_box(ui, e, &currency.code))
            .collect();
        let count = boxes.len();
        view! { <BoardColumn title=title tone=tone count=count>{boxes}</BoardColumn> }
    })
    .collect_view();
    let events = StoredValue::new(view.events.clone());
    let picked = move || {
        let id = ui.selected.get()?;
        let event =
            events.with_value(|all| all.iter().find(|e| e.row.wish.wish_id == id).cloned())?;
        let title = event.row.wish.name.clone();
        let close = Callback::new(move |()| {
            ui.close_forms();
            own.open.set(None);
            ui.selected.set(None);
        });
        let block = event_block(ui, own, &event, months, ctx, &currency);
        Some(view! {
            <Popup title=title class="w-[min(94vw,60rem)]" on_close=close>
                {block}
            </Popup>
        })
    };
    let empty = view.events.is_empty().then(|| {
        view! { <p class="text-sm text-fg-muted">"No events yet."</p> }
    });
    view! {
        <div class="grid gap-4">
            <section aria-label="Events board" class="grid gap-3">
                <div>
                    <Eyebrow>"EVENTS"</Eyebrow>
                    <h2 class="mt-1 text-lg font-semibold">"Planned days and trips"</h2>
                </div>
                <div class="grid items-start gap-3 md:grid-cols-3">{board}</div>
                {empty}
            </section>
            {picked}
            <Card>
                <Eyebrow>"NEW EVENT"</Eyebrow>
                {form}
            </Card>
        </div>
    }
}

/// What an event has used of its plan, 0–100. What is spent already holds
/// what was paid ahead.
pub fn used_percent(e: &EventRow) -> u32 {
    crate::pages::wishes::saved_percent(e.spent, planned_total(e))
}

/// An event as a box on the board: name, plan, days, how it stands, what
/// is used, and its parts as small boxes.
fn event_box(ui: Ui, event: &EventRow, code: &str) -> impl IntoView + use<> {
    let wish = &event.row.wish;
    let id = wish.wish_id.clone();
    let m = |value: Decimal| amount(value, code);
    let days = wish.event.as_ref().map_or_else(String::new, |d| {
        if d.starts_on == d.ends_on {
            format::day_month(&d.starts_on)
        } else {
            format!(
                "{} – {}",
                format::day_month(&d.starts_on),
                format::day_month(&d.ends_on)
            )
        }
    });
    let chip = standing(event).or_else(|| {
        Some(if event.row.short > Decimal::ZERO {
            (Tone::Warning, "Short")
        } else if event.row.tradeoffs {
            (Tone::Warning, "With trade-offs")
        } else {
            (Tone::Positive, "Affordable")
        })
    });
    let facts = match event.phase {
        EventPhase::Upcoming => vec![("Extra to find".to_owned(), m(event.extra))],
        EventPhase::Running => {
            let mut rows = vec![("Left".to_owned(), m(event.left))];
            if event.left > Decimal::ZERO {
                rows.push(("A day".to_owned(), m(event.per_day)));
            }
            rows
        }
        EventPhase::Past => vec![("Spent".to_owned(), m(event.spent))],
    };
    let percent = used_percent(event);
    let used = m(event.spent);
    let parts = (!event.row.children.is_empty()).then(|| {
        let boxes = event
            .row
            .children
            .iter()
            .map(|part| {
                let bought = part.wish.status == finance_types::gateway::WishStatus::Bought;
                let cost = part.wish.bought.as_ref().map_or(part.wish.amount, |p| p.amount);
                view! {
                    <li class="flex items-baseline justify-between gap-2 rounded-sm bg-soft px-2 py-1 text-xs">
                        <span class=cn(["min-w-0 truncate", if bought { "text-fg-muted line-through" } else { "" }])>
                            {part.wish.name.clone()}
                        </span>
                        <span class="whitespace-nowrap tabular-nums">{m(cost)}</span>
                    </li>
                }
            })
            .collect_view();
        view! { <ul class="grid gap-1">{boxes}</ul> }
    });
    let total = m(planned_total(event));
    view! {
        <button
            type="button"
            class=cn([BOARD_CARD, "text-left transition-colors hover:border-over"])
            on:click=move |_| {
                ui.close_forms();
                ui.selected.set(Some(id.clone()));
            }
        >
            <span class="flex items-baseline justify-between gap-3">
                <strong class="min-w-0 truncate font-semibold">{wish.name.clone()}</strong>
                <strong class="whitespace-nowrap tabular-nums">{total}</strong>
            </span>
            <span class="flex flex-wrap items-center justify-between gap-1.5 text-xs text-fg-muted">
                <span class="flex items-center gap-1.5">
                    {wish.must.then(|| view! { <Badge tone=Tone::Accent class="px-2 py-0">"Must"</Badge> })}
                    {chip.map(|(tone, text)| view! { <Badge tone=tone class="px-2 py-0">{text}</Badge> })}
                </span>
                <span class="whitespace-nowrap">{days}</span>
            </span>
            <span class="grid gap-1">
                <span role="img" aria-label=format!("{used} used") class="block h-1.5 overflow-hidden rounded-pill bg-soft">
                    <i class="block h-full rounded-pill bg-over" style:width=format!("{percent}%")></i>
                </span>
                <span class="flex justify-between text-xs text-fg-muted">
                    <span>{format!("{used} used")}</span>
                    <span class="tabular-nums">{format!("{percent}%")}</span>
                </span>
            </span>
            <Facts rows=facts />
            {parts}
        </button>
    }
}

fn add_form(ui: Ui, own: Own, currency: Currency) -> impl IntoView + use<> {
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = parse_amount(&own.amount.get_untracked()) else {
            ui.set_message("event-add", "Enter what you want to spend.", true);
            return;
        };
        let starts = own.starts.get_untracked();
        let draft = Draft {
            wish_id: None,
            parent_id: None,
            name: own.name.get_untracked().trim().to_owned(),
            amount: value,
            currency: currency.clone(),
            due_on: starts.clone(),
            source: WishSource::Card,
            kind: WishKind::OneOff,
            must: own.must.get_untracked(),
            event: Some(EventDetails {
                starts_on: starts,
                ends_on: own.ends.get_untracked(),
                on_top: own.mode.get_untracked() == "extra",
                ..EventDetails::default()
            }),
        };
        ui.write(
            "event-add".into(),
            "save_wish",
            DraftArgs { draft },
            "Event planned.",
            move || {
                for field in [own.name, own.amount, own.starts, own.ends] {
                    field.set(String::new());
                }
                own.must.set(false);
            },
        );
    };
    view! {
        <form class="mt-4 grid items-start gap-3 sm:grid-cols-2 lg:grid-cols-4" on:submit=submit>
            <Field label="Name" id="event-name">{text_input("event-name".into(), own.name, "text")}</Field>
            <Field label="First day" id="event-starts">{text_input("event-starts".into(), own.starts, "date")}</Field>
            <Field label="Last day" id="event-ends">{text_input("event-ends".into(), own.ends, "date")}</Field>
            <Field label="To spend" id="event-amount" hint="Everything those days, not only what is above the usual.">
                {text_input("event-amount".into(), own.amount, "amount")}
            </Field>
            <Field label="The amount is" id="event-mode" hint="In total: wishes put into the event come out of it. Extra: it is for the days, and those wishes come on top.">
                {mode_select("event-mode".into(), own.mode)}
            </Field>
            <label class="flex items-center gap-2.5 text-sm sm:col-start-1">
                <Checkbox
                    checked=Signal::derive(move || own.must.get())
                    on_change=Callback::new(move |v| own.must.set(v))
                />
                "Must happen"
            </label>
            <Button submit=true variant=Variant::Secondary class="sm:col-start-1" disabled=Signal::derive(move || ui.busy.get())>
                <Lucide icon=Icon::Plus size=15 />
                "Plan event"
            </Button>
        </form>
        {feedback(ui, "event-add".into())}
    }
}

/// One event: its standing and numbers, the wish row it is, its payments
/// and the forms that add a part, a payment or new dates.
fn event_block(
    ui: Ui,
    own: Own,
    event: &EventRow,
    months: StoredValue<Vec<WishMonth>>,
    ctx: StoredValue<Context>,
    currency: &Currency,
) -> impl IntoView + use<> {
    let code = currency.code.clone();
    let wish = event.row.wish.clone();
    let id = StoredValue::new(wish.wish_id.clone());
    let target = format!("event:{}", wish.wish_id);
    let chip = standing(event).map(|(tone, text)| view! { <Badge tone=tone>{text}</Badge> });
    let facts = event_facts(event, &code);
    let upcoming = event.phase == EventPhase::Upcoming;
    let past = event.phase == EventPhase::Past;
    let details = wish.event.clone().unwrap_or_default();

    let payments = event
        .transactions
        .iter()
        .map(|t| {
            let tx = t.transaction_id.clone();
            let target = target.clone();
            let remove = move |_| {
                ui.write(
                    target.clone(),
                    "set_event_transaction",
                    PickArgs {
                        wish_id: id.get_value(),
                        transaction_id: tx.clone(),
                        pick: EventPick::Exclude,
                    },
                    "Taken out of the event.",
                    || {},
                );
            };
            let note = if t.part_id.is_some() {
                "A part"
            } else if t.picked {
                "Picked in"
            } else {
                ""
            };
            // A bought part's payment leaves with the part, not from here.
            let action = t.part_id.is_none().then(|| {
                view! {
                    <Button size=Size::Sm variant=Variant::Ghost disabled=Signal::derive(move || ui.busy.get()) on_click=Callback::new(remove)>
                        "Not part of this"
                    </Button>
                }
            });
            view! {
                <li class="grid grid-cols-[6.5rem_1fr_auto_auto] items-center gap-x-3 border-t border-line py-1.5 text-sm first:border-t-0">
                    <span class="text-fg-muted tabular-nums">{format::date(&t.on)}</span>
                    <span class="min-w-0 truncate">
                        {t.description.clone()}
                        {(!note.is_empty()).then(|| view! { <span class="ml-2 text-xs text-fg-muted">{note}</span> })}
                    </span>
                    <span class="tabular-nums">{amount(t.amount, &code)}</span>
                    <span class="min-w-[7.5rem] text-right">{action}</span>
                </li>
            }
        })
        .collect_view();
    let has_payments = !event.transactions.is_empty();
    let body = wish_row(ui, event.row.clone(), months, ctx);

    // Add a part: a sub-wish of the event.
    let part_target = target.clone();
    let part_currency = currency.clone();
    let part_default_day = details.starts_on.clone();
    let add_part = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = parse_amount(&own.part_amount.get_untracked()) else {
            ui.set_message(&part_target, "Enter what the part costs.", true);
            return;
        };
        let draft = Draft {
            wish_id: None,
            parent_id: Some(id.get_value()),
            name: own.part_name.get_untracked().trim().to_owned(),
            amount: value,
            currency: part_currency.clone(),
            due_on: own.part_on.get_untracked(),
            source: WishSource::Card,
            kind: WishKind::OneOff,
            must: false,
            event: None,
        };
        ui.write(
            part_target.clone(),
            "save_wish",
            DraftArgs { draft },
            "Part added.",
            move || {
                own.part_name.set(String::new());
                own.part_amount.set(String::new());
                own.open.set(None);
            },
        );
    };

    // Add a payment: the shared picker, leaving out what the event holds.
    let pay_ask = RwSignal::new(None::<Ask>);
    let ask = StoredValue::new(Ask {
        id: wish.wish_id.clone(),
        name: wish.name.clone(),
        currency: code.clone(),
        incoming: false,
        amount: Decimal::ZERO,
        taken: Some(
            event
                .transactions
                .iter()
                .map(|t| t.transaction_id.clone())
                .collect(),
        ),
    });
    let open_pay = move |_| {
        own.open.set(None);
        pay_ask.set(Some(ask.get_value()));
    };
    let pick_target = target.clone();
    let on_pick = Callback::new(move |(a, row): (Ask, SearchRow)| {
        ui.write(
            pick_target.clone(),
            "set_event_transaction",
            PickArgs {
                wish_id: a.id,
                transaction_id: row.transaction_id,
                pick: EventPick::Pick,
            },
            "Payment added.",
            move || pay_ask.set(None),
        );
    });

    // New dates, amount or way of counting; or back to a plain wish while
    // it has not started.
    let dates_target = target.clone();
    let kept = StoredValue::new(wish.clone());
    let save_dates = move |ev: SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = parse_amount(&own.new_amount.get_untracked()) else {
            ui.set_message(&dates_target, "Enter what you want to spend.", true);
            return;
        };
        let wish = kept.get_value();
        let starts = own.new_starts.get_untracked();
        let draft = Draft {
            wish_id: Some(wish.wish_id),
            parent_id: None,
            name: wish.name,
            amount: value,
            currency: wish.currency,
            due_on: starts.clone(),
            source: wish.source,
            kind: wish.kind,
            must: wish.must,
            event: Some(EventDetails {
                starts_on: starts,
                ends_on: own.new_ends.get_untracked(),
                on_top: own.new_mode.get_untracked() == "extra",
                ..EventDetails::default()
            }),
        };
        ui.write(
            dates_target.clone(),
            "save_wish",
            DraftArgs { draft },
            "Saved.",
            move || own.open.set(None),
        );
    };
    let back_target = target.clone();
    let back = move |_| {
        ui.write(
            back_target.clone(),
            "wish_back_from_event",
            WishIdArgs {
                wish_id: id.get_value(),
            },
            "Back among the wishes.",
            || {},
        );
    };
    let planned_amount = wish.amount.normalize().to_string();
    let extra = details.on_top;
    let open_dates = {
        let (starts, ends) = (details.starts_on.clone(), details.ends_on.clone());
        move |_| {
            own.new_starts.set(starts.clone());
            own.new_ends.set(ends.clone());
            own.new_amount.set(planned_amount.clone());
            own.new_mode
                .set(if extra { "extra" } else { "total" }.into());
            id.with_value(|id| own.toggle(id, "dates"));
        }
    };
    let open_part = move |_| {
        if own.part_on.get_untracked().is_empty() {
            own.part_on.set(part_default_day.clone().max(today()));
        }
        id.with_value(|id| own.toggle(id, "part"));
    };
    let is = move |form: &'static str| id.with_value(|id| own.is_open(id, form));
    let ids = StoredValue::new(wish.wish_id.clone());
    let field_id = move |name: &str| format!("{name}-{}", ids.get_value());
    let add_part = StoredValue::new(add_part);
    let save_dates = StoredValue::new(save_dates);

    view! {
        <div>
            <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
                {chip}
                <FactsRow rows=facts />
            </div>
            <div class="mt-3">{body}</div>
            {has_payments.then(|| view! {
                <div class="mt-2">
                    <Eyebrow>"PAYMENTS"</Eyebrow>
                    <ul class="mt-1">{payments}</ul>
                </div>
            })}
            <div class="mt-3 flex flex-wrap gap-2">
                {(!past).then(|| view! {
                    <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(open_part)>"Add a part"</Button>
                })}
                <Button
                    size=Size::Sm
                    variant=Variant::Secondary
                    on_click=Callback::new(open_pay)
                >
                    "Add a payment"
                </Button>
                <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(open_dates)>"Change dates or amount"</Button>
                {upcoming.then(|| view! {
                    <Button size=Size::Sm variant=Variant::Ghost disabled=Signal::derive(move || ui.busy.get()) on_click=Callback::new(back)>
                        "Back to wishes"
                    </Button>
                })}
            </div>
            <Show when=move || is("part")>
                <form
                    class="mt-3 grid items-end gap-2 sm:grid-cols-4"
                    on:submit=move |ev: SubmitEvent| add_part.with_value(|f| f(ev))
                >
                    <Field label="Part" id=field_id("part-name")>{text_input(field_id("part-name"), own.part_name, "text")}</Field>
                    <Field label="Costs" id=field_id("part-amount")>{text_input(field_id("part-amount"), own.part_amount, "amount")}</Field>
                    <Field label="By" id=field_id("part-on")>{text_input(field_id("part-on"), own.part_on, "date")}</Field>
                    <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Add part"</Button>
                </form>
            </Show>
            <PaymentPicker
                ask=pay_ask
                busy=Signal::derive(move || ui.busy.get())
                on_pick=on_pick
                otherwise="Newest first. Type a shop, a name or a description to narrow them."
            />
            <Show when=move || is("dates")>
                <form
                    class="mt-3 grid items-end gap-2 sm:grid-cols-4"
                    on:submit=move |ev: SubmitEvent| save_dates.with_value(|f| f(ev))
                >
                    <Field label="First day" id=field_id("new-starts")>{text_input(field_id("new-starts"), own.new_starts, "date")}</Field>
                    <Field label="Last day" id=field_id("new-ends")>{text_input(field_id("new-ends"), own.new_ends, "date")}</Field>
                    <Field label="To spend" id=field_id("new-amount")>{text_input(field_id("new-amount"), own.new_amount, "amount")}</Field>
                    <Field label="The amount is" id=field_id("new-mode")>{mode_select(field_id("new-mode"), own.new_mode)}</Field>
                    <Button submit=true size=Size::Sm disabled=Signal::derive(move || ui.busy.get())>"Save"</Button>
                </form>
            </Show>
            {feedback(ui, target)}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::gateway::{Wish, WishStatus};
    use finance_types::report::wishes::WishRow;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn row(id: &str, amount: &str, parent: Option<&str>) -> WishRow {
        WishRow {
            wish: Wish {
                wish_id: id.into(),
                parent_id: parent.map(str::to_owned),
                name: id.into(),
                amount: d(amount),
                currency: Currency::new("EUR"),
                due_on: "2026-11-30".into(),
                source: WishSource::Card,
                saved: Decimal::ZERO,
                saved_month: None,
                status: WishStatus::Planned,
                bought: None,
                kind: WishKind::OneOff,
                must: false,
                event: None,
                created_at: String::new(),
            },
            children: Vec::new(),
            rest: Decimal::ZERO,
            over_cap: false,
            monthly_set_aside: Decimal::ZERO,
            suggested_saved: Decimal::ZERO,
            short: Decimal::ZERO,
            tradeoffs: false,
            not_judged: None,
        }
    }

    fn event(phase: EventPhase) -> EventRow {
        EventRow {
            row: row("Parents", "400", None),
            phase,
            usual: d("150"),
            extra: d("250"),
            paid_ahead: Decimal::ZERO,
            spent: Decimal::ZERO,
            left: d("400"),
            days: 6,
            days_left: 0,
            per_day: Decimal::ZERO,
            pace: Pace::OnTrack,
            fits_up_to: Some(d("520")),
            months: Vec::new(),
            transactions: Vec::new(),
        }
    }

    fn labels(facts: &[Fact]) -> Vec<&str> {
        facts.iter().map(|(label, _)| label.as_str()).collect()
    }

    #[test]
    fn an_upcoming_event_shows_what_it_costs_and_not_what_is_spent() {
        let facts = event_facts(&event(EventPhase::Upcoming), "EUR");
        assert_eq!(
            labels(&facts),
            [
                "Planned",
                "Usual those days",
                "Extra to find",
                "Fits without trade-offs"
            ]
        );
        assert_eq!(standing(&event(EventPhase::Upcoming)), None);
    }

    #[test]
    fn an_amount_on_top_of_the_parts_adds_them_to_the_plan() {
        let mut e = event(EventPhase::Upcoming);
        let own = e.row.wish.amount;
        e.row.children.push(row("Zoo", "18", Some("Parents")));
        // In total: the part comes out of the amount.
        assert_eq!(planned_total(&e), own);
        e.row.wish.event = Some(EventDetails {
            on_top: true,
            ..EventDetails::default()
        });
        assert_eq!(planned_total(&e), own + d("18"));
    }

    #[test]
    fn a_running_event_shows_its_pace_and_a_day_only_while_money_is_left() {
        let mut running = EventRow {
            spent: d("187.60"),
            left: d("166.40"),
            days_left: 4,
            per_day: d("41.60"),
            paid_ahead: d("28"),
            ..event(EventPhase::Running)
        };
        running.row.children.push(row("Zoo", "18", Some("Parents")));
        let facts = event_facts(&running, "EUR");
        assert_eq!(
            labels(&facts),
            [
                "Planned",
                "Parts planned",
                "Paid ahead",
                "Usual those days",
                "Extra to find",
                "Spent",
                "Left",
                "Days left",
                "A day, to stay inside"
            ]
        );
        assert_eq!(facts[8].1, "€41.60");
        let over = EventRow {
            left: d("-20"),
            pace: Pace::Over,
            ..running
        };
        assert!(!labels(&event_facts(&over, "EUR")).contains(&"A day, to stay inside"));
        assert_eq!(standing(&over), Some((Tone::Warning, "Over")));
    }
}
