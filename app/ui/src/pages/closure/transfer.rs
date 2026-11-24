//! Marking a transfer done by hand (personal-site
//! `components/month-closure/transfer-mark.tsx`).

use std::collections::{HashMap, HashSet};

use finance_types::report::closure::{AllocationLine, TransferStatus};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::Wizard;
use super::ui::{ACTIONS, CALLOUT, FOOTNOTE, SMALL, WARN, money};
use crate::ds::patterns::STACK_SM;
use crate::ds::{Button, Checkbox, Size, Variant, cn};

/// A routed line's hop index, or the direct transfer of an unrouted line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hop {
    Direct,
    Index(u32),
}

impl Hop {
    pub fn index(self) -> Option<u32> {
        match self {
            Self::Direct => None,
            Self::Index(i) => Some(i),
        }
    }
}

/// One bank move: a hop of a routed line, or a direct line's transfer.
#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    pub from: String,
    pub to: Option<String>,
    pub amount: Decimal,
    pub transfer_status: TransferStatus,
    pub matched_transaction_ids: Vec<String>,
    pub manual: bool,
    pub moved_amount: Option<Decimal>,
}

pub fn legs_of(line: &AllocationLine) -> Vec<Leg> {
    if line.transfer_hops.is_empty() {
        return vec![Leg {
            from: line
                .transfer_source_account_id
                .clone()
                .unwrap_or_else(|| line.from_account_id.clone()),
            to: line.to_account_id.clone(),
            amount: line.amount.abs(),
            transfer_status: line.transfer_status,
            matched_transaction_ids: line.matched_transaction_ids.clone(),
            manual: line.manual,
            moved_amount: line.moved_amount,
        }];
    }
    line.transfer_hops
        .iter()
        .map(|hop| Leg {
            from: hop.from_account_id.clone(),
            to: Some(hop.to_account_id.clone()),
            amount: hop.amount,
            transfer_status: hop.transfer_status,
            matched_transaction_ids: hop.matched_transaction_ids.clone(),
            manual: hop.manual,
            moved_amount: hop.moved_amount,
        })
        .collect()
}

/// What every leg sharing these bank transfers needed, so a €500 top-up
/// covering two €250 hops is no mismatch.
fn needed(leg: &Leg, lines: &[AllocationLine]) -> Decimal {
    let ids: HashSet<&str> = leg
        .matched_transaction_ids
        .iter()
        .map(String::as_str)
        .collect();
    lines
        .iter()
        .filter(|l| l.transfer_status != TransferStatus::NotNeeded)
        .flat_map(legs_of)
        .filter(|other| {
            other
                .matched_transaction_ids
                .iter()
                .any(|id| ids.contains(id.as_str()))
        })
        .map(|other| other.amount)
        .sum()
}

#[component]
pub fn LegStatus(
    line: AllocationLine,
    leg: Leg,
    hop: Hop,
    names: StoredValue<HashMap<String, String>>,
    actions: bool,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let disabled = Signal::derive(move || wizard.busy.get());
    let account =
        move |id: &str| names.with_value(|n| n.get(id).cloned().unwrap_or_else(|| id.to_owned()));
    let picking = RwSignal::new(false);
    let picked = RwSignal::new(Vec::<String>::new());
    let matched = leg.transfer_status == TransferStatus::Matched;
    let unit = line.currency.clone();
    let view_now = wizard.view.get_untracked();
    let (all_lines, candidates) = view_now
        .map(|v| (v.period.allocations, v.transfer_candidates))
        .unwrap_or_default();
    let required = (matched && leg.manual && leg.moved_amount.is_some() && actions)
        .then(|| needed(&leg, &all_lines));
    let short = match (required, leg.moved_amount) {
        (Some(required), Some(moved)) if required != moved => Some((moved, required)),
        _ => None,
    };
    let from_name = account(&leg.from);
    let to_name = leg.to.as_deref().map_or_else(|| "?".to_owned(), account);
    let choices: Vec<_> = candidates
        .into_iter()
        .filter(|c| {
            c.from_account_id == leg.from
                && c.to_account_id == leg.to
                && c.currency.code == line.currency.code
        })
        .collect();
    let status = if matched {
        if leg.manual {
            "Marked done by hand"
        } else {
            "Found in bank data"
        }
    } else {
        "Waiting for transfer"
    };
    let line_id = StoredValue::new(line.line_id.clone());
    let picker = move || {
        let unit = unit.clone();
        let list = if choices.is_empty() {
            view! {
                <p class=FOOTNOTE>
                    {format!("No bank transfer from {from_name} to {to_name} since the split yet. Sync banks after making it.")}
                </p>
            }
            .into_any()
        } else {
            let items = choices
                .iter()
                .map(|c| {
                    let key = c.transaction_ids.join(",");
                    let k1 = key.clone();
                    let k2 = key.clone();
                    let text = format!(
                        "{} · {}",
                        c.booked_on,
                        money(c.amount, &unit)
                    );
                    let description = c
                        .description
                        .as_deref()
                        .map(|d| format!(" · {d}"))
                        .unwrap_or_default();
                    view! {
                        <li>
                            <label class="flex cursor-pointer items-start gap-2">
                                <span class="mt-0.5">
                                    <Checkbox
                                        checked=Signal::derive(move || picked.with(|p| p.contains(&k1)))
                                        on_change=Callback::new(move |on: bool| picked.update(|p| {
                                            if on {
                                                p.push(k2.clone());
                                            } else {
                                                p.retain(|item| *item != k2);
                                            }
                                        }))
                                    />
                                </span>
                                <span class="min-w-0"><span class="tabular-nums">{text}</span>{description}</span>
                            </label>
                        </li>
                    }
                })
                .collect_view();
            view! {
                <p class=FOOTNOTE>
                    {format!("Bank transfers from {from_name} to {to_name} since the split. Pick the ones that made this move.")}
                </p>
                <ul class=STACK_SM>{items}</ul>
            }
            .into_any()
        };
        view! {
            <div class=cn([CALLOUT, "basis-full"]) role="group" aria-label=format!("Transfers from {from_name} to {to_name}")>
                {list}
                <div class=ACTIONS>
                    <Button
                        size=Size::Sm
                        disabled=Signal::derive(move || disabled.get() || picked.with(Vec::is_empty))
                        on_click=Callback::new(move |_| {
                            let ids: Vec<String> = picked
                                .get_untracked()
                                .iter()
                                .flat_map(|k| k.split(',').map(str::to_owned))
                                .collect();
                            wizard.mark(line_id.get_value(), hop, ids, picking);
                        })
                    >
                        "Mark done"
                    </Button>
                    <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| picking.set(false))>"Cancel"</Button>
                </div>
            </div>
        }
    };
    let picker = StoredValue::new_local(picker);
    view! {
        <small class=SMALL>{status}</small>
        {short.map(|(moved, required)| view! {
            <small class=cn([WARN, "basis-full text-xs"])>
                {format!(
                    "moved {} of {} — edit the split if this is the real amount",
                    money(moved, &line.currency),
                    money(required, &line.currency),
                )}
            </small>
        })}
        {(actions && matched && leg.manual).then(|| view! {
            <Button size=Size::Sm variant=Variant::Ghost disabled=disabled on_click=Callback::new(move |_| {
                wizard.unmark(line_id.get_value(), hop);
            })>"Unmark"</Button>
        })}
        {(actions && !matched).then(|| view! {
            {move || if picking.get() {
                picker.with_value(|f| f()).into_any()
            } else {
                view! {
                    <Button size=Size::Sm variant=Variant::Secondary disabled=disabled on_click=Callback::new(move |_| {
                        picked.set(Vec::new());
                        picking.set(true);
                    })>"Mark done…"</Button>
                }
                .into_any()
            }}
        })}
    }
}
