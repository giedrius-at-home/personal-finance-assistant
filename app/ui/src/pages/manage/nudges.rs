//! Manage → Settings: which spending groups count as avoidable in the
//! evening overspend summary, and when in the day purchases happen (which
//! sets the extra bank syncs).

use finance_types::bank::PurchaseTiming;
use finance_types::notify::AvoidableGroup;
use leptos::prelude::*;
use serde::Serialize;

use super::ui::{Messages, Ops};
use crate::api::{NoArgs, load};
use crate::ds::patterns::{
    CHECK_SLOT, HINT, LIST_STACK, MUTED, OPTION, SMALL, STATUS_NOTE, SUBPANEL,
};
use crate::ds::{Card, CardTitle, Checkbox, Eyebrow, Heading};

#[derive(Serialize)]
struct TickArgs {
    group: String,
    avoidable: Option<bool>,
}

#[component]
pub fn AvoidableCard() -> impl IntoView {
    let ops = Ops::new();
    let busy = ops.busy;
    let changed = RwSignal::new(0_u32);
    let groups = load::<Vec<AvoidableGroup>, _>("avoidable_groups", move || {
        changed.track();
        NoArgs {}
    });
    let tick = move |group: String, avoidable: bool| {
        ops.run(
            "set_avoidable_group",
            TickArgs {
                group,
                avoidable: Some(avoidable),
            },
            move |_: Vec<AvoidableGroup>| changed.update(|n| *n += 1),
        );
    };
    let list = move |rows: Vec<AvoidableGroup>| {
        if rows.is_empty() {
            return view! { <p class=STATUS_NOTE>"No spending groups yet."</p> }.into_any();
        }
        view! {
            <div class="mt-4 grid gap-2 sm:grid-cols-2">
                {rows
                    .into_iter()
                    .map(|row| {
                        let group = row.group.clone();
                        view! {
                            <label class=OPTION>
                                <span class=CHECK_SLOT>
                                    <Checkbox
                                        checked=Signal::derive(move || row.avoidable)
                                        disabled=Signal::derive(move || busy.get())
                                        on_change=Callback::new(move |v| tick(group.clone(), v))
                                    />
                                </span>
                                <span>
                                    {row.group.clone()}
                                    {row.default.then(|| view! { <span class=SMALL>"default"</span> })}
                                </span>
                            </label>
                        }
                    })
                    .collect_view()}
            </div>
        }
        .into_any()
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"SUMMARY"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Avoidable spending"</CardTitle>
            <p class=MUTED>
                "Purchases in ticked groups are named in the evening summary on a day that went over, each with something to do instead. Defaults come from the group's name; groceries, rent and bills never count."
            </p>
            <div class=LIST_STACK>
                <Messages ops=ops />
            </div>
            {move || match groups.data.get() {
                None => view! { <p class=STATUS_NOTE>"Loading groups…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class=STATUS_NOTE>{format!("Groups are not available ({e}).")}</p> }.into_any(),
                Some(Ok(rows)) => list(rows),
            }}
        </Card>
    }
}

/// The 24 hours as a row of bars, busiest darkest.
fn hour_bars(hours: Vec<u32>) -> impl IntoView {
    let peak = hours.iter().copied().max().unwrap_or(0).max(1);
    view! {
        <div class="mt-3 flex h-16 items-end gap-px" aria-hidden="true">
            {hours
                .into_iter()
                .map(|h| {
                    let pct = (u64::from(h) * 100 / u64::from(peak)).max(2);
                    view! { <span class="flex-1 rounded-t-sm bg-accent" style=format!("height:{pct}%") /> }
                })
                .collect_view()}
        </div>
        <div class="flex justify-between text-xs text-fg-muted">
            <span>"0:00"</span>
            <span>"6:00"</span>
            <span>"12:00"</span>
            <span>"18:00"</span>
            <span>"23:00"</span>
        </div>
    }
}

#[component]
pub fn TimingCard() -> impl IntoView {
    let timing = load::<PurchaseTiming, _>("purchase_timing", || NoArgs {});
    let body = |t: PurchaseTiming| {
        let peaks = t
            .peaks
            .iter()
            .map(|h| format!("{h}:00"))
            .collect::<Vec<_>>()
            .join(", ");
        let basis = if t.learned {
            format!(
                "From {} card purchases with the time on the receipt and {} timed by when a sync first saw them.",
                t.exact, t.estimated
            )
        } else {
            format!(
                "Not enough history yet ({} timed purchases), so the usual lunch, after-work and evening hours are used.",
                t.exact + t.estimated
            )
        };
        view! {
            <div class=SUBPANEL>
                <p class=MUTED>{format!("You usually buy around {peaks}.")}</p>
                {hour_bars(t.hours.clone())}
                <p class=HINT>{basis}</p>
                <p class=MUTED>
                    {format!(
                        "Extra bank syncs today at {}, besides the daily one (at most four a day per bank).",
                        t.slots.join(", ")
                    )}
                </p>
            </div>
        }
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"BANKS"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"When you buy"</CardTitle>
            <p class=MUTED>
                "Banks report dates only. Card receipts from Swedbank and ABN AMRO carry the time; other purchases are placed between two syncs. The busiest hours get an extra sync soon after, so alerts arrive sooner."
            </p>
            {move || match timing.data.get() {
                None => view! { <p class=STATUS_NOTE>"Loading…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class=STATUS_NOTE>{format!("Not available ({e}).")}</p> }.into_any(),
                Some(Ok(t)) => body(t).into_any(),
            }}
        </Card>
    }
}
