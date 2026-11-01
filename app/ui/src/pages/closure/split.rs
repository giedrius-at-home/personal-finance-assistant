//! The split editor and its lines (personal-site
//! `components/month-closure/split-editor.tsx` and `split-line.tsx`).

use std::collections::{BTreeMap, HashMap};

use finance_types::report::Currency;
use finance_types::report::closure::{
    AllocationLine, LineKind, PotView, SavingsSuggestion, TransferStatus,
};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::Wizard;
use super::suggestion::{SuggestionLine, SuggestionPanel, figures};
use super::ui::{
    ACTIONS, Choice, FieldBox, GROW, INLINE_FORM, SMALL, TextBox, WARN, decimal_places, field_id,
    fixed, minor_amount, money, new_id, next_month, options, parse_amount,
};
use crate::ds::{Button, Size, Variant, cn};

/// One line as typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub key: String,
    pub destination: String,
    /// As typed. A take is typed positive and saved negative.
    pub amount: String,
    /// A take from a pot: money arriving on this account.
    pub take: bool,
    pub plan_name: String,
    /// YYYY-MM; saved as the first day of that month.
    pub plan_month: Option<String>,
    pub plan_category: String,
    pub plan_shop: String,
    pub plan_id: Option<String>,
}

/// The drafts of one split, kept by the wizard so a reload of the month
/// (a new pot, say) does not lose what was typed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DraftStore {
    pub key: String,
    pub accounts: BTreeMap<String, Vec<Draft>>,
}

/// An account whose amount must be assigned.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitTarget {
    pub account_id: String,
    pub name: String,
    pub currency: Currency,
    /// The leftover (settle) or the leftover change (resettle).
    pub target: Decimal,
    pub note: Option<String>,
}

/// The signed amount a draft stands for, or `None` when it is not a valid
/// amount for its line.
fn parse(draft: &Draft, minor_unit: u8, allow_negative: bool) -> Option<Decimal> {
    let value = parse_amount(draft.amount.trim())?;
    if decimal_places(value) > u32::from(minor_unit) {
        return None;
    }
    if draft.take {
        return (value > Decimal::ZERO).then_some(-value);
    }
    if draft.destination == "plan" {
        return (value > Decimal::ZERO).then_some(value);
    }
    let bad = if allow_negative {
        value.is_zero()
    } else {
        value <= Decimal::ZERO
    };
    (!bad).then_some(value)
}

fn amount_of(draft: &Draft) -> Decimal {
    parse_amount(draft.amount.trim()).unwrap_or_default()
}

fn draft_from(line: &AllocationLine) -> Draft {
    let take = line.pot_id().is_some() && line.amount.is_sign_negative();
    let amount = minor_amount(&line.amount.to_string(), line.currency.minor_unit);
    let plan = line.destination == "plan";
    Draft {
        key: line.line_id.clone(),
        destination: line.destination.clone(),
        amount: if take {
            amount.trim_start_matches('-').to_owned()
        } else {
            amount
        },
        take,
        plan_name: if plan {
            line.plan_name.clone().unwrap_or_default()
        } else {
            String::new()
        },
        plan_month: if plan {
            line.plan_due_on
                .as_deref()
                .and_then(|d| d.get(..7))
                .map(str::to_owned)
        } else {
            None
        },
        plan_category: if plan {
            line.plan_match_category.clone().unwrap_or_default()
        } else {
            String::new()
        },
        plan_shop: if plan {
            line.plan_match_description.clone().unwrap_or_default()
        } else {
            String::new()
        },
        plan_id: if plan { line.plan_id.clone() } else { None },
    }
}

/// `Pot name · €12.00` (the balance in the account's currency, if any).
pub fn pot_label(pot: &PotView, currency: &Currency) -> String {
    match pot.balances.get(&currency.code) {
        Some(balance) => format!("{} · {}", pot.name, money(*balance, currency)),
        None => pot.name.clone(),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    account: SplitTarget,
    lines: Vec<Draft>,
    parsed: Vec<Option<Decimal>>,
    valid: bool,
    remaining: Decimal,
    errors: Vec<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
struct Rows {
    rows: Vec<Row>,
    complete: bool,
}

struct Setup {
    accounts: Vec<SplitTarget>,
    pots: Vec<PotView>,
    committed: Vec<AllocationLine>,
    allow_negative: bool,
    min_plan_month: String,
}

fn compute(setup: &Setup, drafts: &DraftStore) -> Rows {
    // What each pot can still give per currency: its balance without this
    // month's saved entries, minus other takes.
    let mut room: HashMap<String, Decimal> = HashMap::new();
    for pot in &setup.pots {
        for (code, balance) in &pot.balances {
            room.insert(format!("{}|{code}", pot.pot_id), *balance);
        }
    }
    for line in &setup.committed {
        if let Some(pot) = line.pot_id() {
            let key = format!("{pot}|{}", line.currency.code);
            let now = room.get(&key).copied().unwrap_or_default();
            room.insert(key, now - line.amount);
        }
    }
    let mut complete = true;
    let rows = setup
        .accounts
        .iter()
        .map(|account| {
            let lines = drafts
                .accounts
                .get(&account.account_id)
                .cloned()
                .unwrap_or_default();
            let unit = account.currency.minor_unit;
            let parsed: Vec<Option<Decimal>> = lines
                .iter()
                .map(|l| parse(l, unit, setup.allow_negative))
                .collect();
            let errors: Vec<Option<String>> = lines
                .iter()
                .zip(&parsed)
                .map(|(line, value)| {
                    if line.destination == "plan" {
                        if line.plan_name.trim().is_empty() {
                            return Some("Name the planned purchase.".into());
                        }
                        if line
                            .plan_month
                            .as_deref()
                            .is_none_or(|m| m.is_empty() || m < setup.min_plan_month.as_str())
                        {
                            return Some("Plan it for a later month.".into());
                        }
                    }
                    if let (true, Some(value)) = (line.take, value) {
                        let key = format!(
                            "{}|{}",
                            line.destination.trim_start_matches("pot:"),
                            account.currency.code
                        );
                        let left = room.get(&key).copied().unwrap_or_default();
                        room.insert(key, left + value);
                        if left + value < Decimal::ZERO {
                            return Some(format!(
                                "More than the pot holds ({} left).",
                                money(left.max(Decimal::ZERO), &account.currency)
                            ));
                        }
                    }
                    None
                })
                .collect();
            let valid = parsed.iter().all(Option::is_some);
            let assigned: Decimal = parsed.iter().flatten().sum();
            let remaining = account.target - assigned;
            if !valid || !remaining.is_zero() || errors.iter().any(Option::is_some) {
                complete = false;
            }
            Row {
                account: account.clone(),
                lines,
                parsed,
                valid,
                remaining,
                errors,
            }
        })
        .collect();
    Rows { rows, complete }
}

/// Per-account allocation lines. Every account's lines must add up exactly
/// to its target before the owner can submit; a take from a pot adds to
/// what the account has to assign. Split mode needs positive amounts except
/// for takes; delta mode (resettle) allows negatives.
#[component]
pub fn SplitEditor(
    accounts: Vec<SplitTarget>,
    goals: Vec<(String, String)>,
    /// Existing split lines to prefill (editing a confirmed split).
    #[prop(optional)]
    initial: Vec<AllocationLine>,
    #[prop(optional)] allow_negative: bool,
    #[prop(into)] submit_label: String,
    on_submit: Callback<Vec<AllocationLine>>,
    /// The month being closed (YYYY-MM-01); planned purchases default to the
    /// month after it.
    #[prop(into)]
    month: String,
    /// Active pots to offer as destinations.
    #[prop(optional)]
    pots: Vec<PotView>,
    /// Lines already saved for this month: their pot entries are replaced on
    /// submit, so takes may reuse them.
    #[prop(optional)]
    committed: Vec<AllocationLine>,
    #[prop(default = None)] suggestion: Option<SavingsSuggestion>,
    /// Known categories a planned purchase can be matched by.
    #[prop(optional)]
    categories: Vec<String>,
    /// Drafts typed under this key survive a reload of the month.
    #[prop(into)]
    store_key: String,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let store = wizard.drafts;
    let disabled = Signal::derive(move || wizard.busy.get());
    if store.with_untracked(|s| s.key != store_key) {
        let accounts_map = accounts
            .iter()
            .map(|account| {
                let existing: Vec<Draft> = initial
                    .iter()
                    .filter(|l| {
                        l.kind == LineKind::Split && l.from_account_id == account.account_id
                    })
                    .map(draft_from)
                    .collect();
                // A deficit account has nothing to split; it starts empty and
                // may only take from a pot.
                let empty = !allow_negative && account.target <= Decimal::ZERO;
                let lines = if !existing.is_empty() || empty {
                    existing
                } else {
                    vec![Draft {
                        key: new_id(),
                        destination: "stay".into(),
                        amount: fixed(account.target, account.currency.minor_unit),
                        ..Draft::default()
                    }]
                };
                (account.account_id.clone(), lines)
            })
            .collect();
        store.set(DraftStore {
            key: store_key,
            accounts: accounts_map,
        });
    }
    let min_plan_month = next_month(&month)[..7].to_owned();
    let setup = StoredValue::new(Setup {
        accounts: accounts.clone(),
        pots: pots.clone(),
        committed,
        allow_negative,
        min_plan_month: min_plan_month.clone(),
    });
    let rows = Memo::new(move |_| store.with(|d| setup.with_value(|s| compute(s, d))));
    let set_account =
        move |account_id: String, change: Box<dyn FnOnce(Vec<Draft>) -> Vec<Draft>>| {
            store.update(|s| {
                let lines = s.accounts.remove(&account_id).unwrap_or_default();
                s.accounts.insert(account_id, change(lines));
            });
        };

    let submit = move |_| {
        let lines: Vec<AllocationLine> = rows.with_untracked(|r| {
            r.rows
                .iter()
                .flat_map(|row| {
                    row.lines.iter().zip(&row.parsed).map(|(line, value)| {
                        let unit = row.account.currency.minor_unit;
                        let amount = value
                            .map(|v| fixed(v, unit).parse().unwrap_or(v))
                            .unwrap_or_default();
                        let plan = line.destination == "plan";
                        let trimmed = |s: &str| {
                            let s = s.trim();
                            (!s.is_empty()).then(|| s.to_owned())
                        };
                        AllocationLine {
                            line_id: line.key.clone(),
                            from_account_id: row.account.account_id.clone(),
                            destination: line.destination.clone(),
                            amount,
                            currency: row.account.currency.clone(),
                            kind: LineKind::Split,
                            transfer_status: TransferStatus::NotNeeded,
                            to_account_id: None,
                            matched_transaction_ids: Vec::new(),
                            plan_name: plan.then(|| line.plan_name.trim().to_owned()),
                            plan_due_on: if plan {
                                line.plan_month.as_ref().map(|m| format!("{m}-01"))
                            } else {
                                None
                            },
                            plan_match_category: if plan {
                                trimmed(&line.plan_category)
                            } else {
                                None
                            },
                            plan_match_description: if plan {
                                trimmed(&line.plan_shop)
                            } else {
                                None
                            },
                            plan_id: if plan { line.plan_id.clone() } else { None },
                            transfer_source_account_id: None,
                            transfer_hops: Vec::new(),
                            manual: false,
                            moved_amount: None,
                        }
                    })
                })
                .collect()
        });
        on_submit.run(lines);
    };

    // Savings suggestion (EUR only, split mode).
    let general = suggestion
        .as_ref()
        .map(|s| format!("pot:{}", s.general_pot_id))
        .unwrap_or_default();
    let general = StoredValue::new(general);
    let eur_rows = move || -> Vec<Row> {
        rows.with_untracked(|r| {
            r.rows
                .iter()
                .filter(|row| row.account.currency.code == "EUR")
                .cloned()
                .collect()
        })
    };
    // Moves up to `amount` from the account with the most unassigned or
    // staying money into general savings.
    let apply_suggestion = Callback::new(move |amount: Decimal| {
        let mut free: Vec<(Row, Decimal)> = eur_rows()
            .into_iter()
            .map(|row| {
                let staying: Decimal = row
                    .lines
                    .iter()
                    .filter(|l| l.destination == "stay" && !l.take)
                    .map(amount_of)
                    .sum();
                let free = row.remaining.max(Decimal::ZERO) + staying;
                (row, free)
            })
            .collect();
        free.sort_by_key(|a| std::cmp::Reverse(a.1));
        let Some((row, free)) = free.into_iter().next() else {
            return;
        };
        if free <= Decimal::ZERO {
            return;
        }
        let moved = amount.min(free);
        let mut from_stay = moved - row.remaining.max(Decimal::ZERO);
        let unit = row.account.currency.minor_unit;
        let general = general.get_value();
        let mut next: Vec<Draft> = Vec::new();
        for line in row.lines {
            if from_stay <= Decimal::ZERO || line.destination != "stay" || line.take {
                next.push(line);
                continue;
            }
            let used = from_stay.min(amount_of(&line));
            from_stay -= used;
            let left = amount_of(&line) - used;
            if left > Decimal::ZERO {
                next.push(Draft {
                    amount: fixed(left, unit),
                    ..line
                });
            }
        }
        if let Some(existing) = next
            .iter_mut()
            .find(|l| l.destination == general && !l.take)
        {
            existing.amount = fixed(amount_of(existing) + moved, unit);
        } else {
            next.push(Draft {
                key: new_id(),
                destination: general,
                amount: fixed(moved, unit),
                ..Draft::default()
            });
        }
        set_account(row.account.account_id, Box::new(move |_| next));
    });
    // Takes `amount` from general savings onto the account with the largest
    // leftover; it stays there for next month.
    let take_from_general = Callback::new(move |amount: Decimal| {
        let mut list = eur_rows();
        list.sort_by_key(|a| std::cmp::Reverse(a.account.target));
        let Some(row) = list.into_iter().next() else {
            return;
        };
        let unit = row.account.currency.minor_unit;
        let general = general.get_value();
        set_account(
            row.account.account_id,
            Box::new(move |mut lines| {
                let grow =
                    |lines: &mut Vec<Draft>, matches: &dyn Fn(&Draft) -> bool, create: Draft| {
                        if let Some(found) = lines.iter_mut().find(|l| matches(l)) {
                            found.amount = fixed(amount_of(found) + amount, unit);
                        } else {
                            lines.push(create);
                        }
                    };
                let g = general.clone();
                grow(
                    &mut lines,
                    &|l| l.take && l.destination == g,
                    Draft {
                        key: new_id(),
                        destination: general.clone(),
                        amount: fixed(amount, unit),
                        take: true,
                        ..Draft::default()
                    },
                );
                grow(
                    &mut lines,
                    &|l| l.destination == "stay" && !l.take,
                    Draft {
                        key: new_id(),
                        destination: "stay".into(),
                        amount: fixed(amount, unit),
                        ..Draft::default()
                    },
                );
                lines
            }),
        );
    });

    let suggestion_open = RwSignal::new(false);
    let suggestion = StoredValue::new(suggestion);
    let has_eur = accounts.iter().any(|a| a.currency.code == "EUR");
    let suggestion_view = move || {
        let inputs = suggestion.get_value()?;
        if allow_negative || !has_eur {
            return None;
        }
        let lines: Vec<SuggestionLine> = rows.with(|r| {
            r.rows
                .iter()
                .flat_map(|row| {
                    row.lines
                        .iter()
                        .zip(&row.parsed)
                        .filter_map(|(line, value)| {
                            value.map(|amount| SuggestionLine {
                                destination: line.destination.clone(),
                                amount,
                                currency: row.account.currency.code.clone(),
                            })
                        })
                })
                .collect()
        });
        let figures = figures(&inputs, &lines);
        Some(view! {
            <SuggestionPanel
                inputs=inputs
                figures=figures
                disabled=disabled
                on_apply=apply_suggestion
                on_take=take_from_general
                open=suggestion_open
            />
        })
    };

    let goals = StoredValue::new(goals);
    let pots = StoredValue::new(pots);
    let categories = StoredValue::new(categories);
    let min_plan_month = StoredValue::new(min_plan_month);
    let fieldsets = accounts
        .into_iter()
        .enumerate()
        .map(|(index, account)| {
            let account = StoredValue::new(account);
            let account_id = account.with_value(|a| a.account_id.clone());
            let row = move || rows.with(|r| r.rows.get(index).cloned());
            let each_id = account_id.clone();
            let lines = move || {
                store.with(|s| s.accounts.get(&each_id).cloned().unwrap_or_default())
            };
            let add_id = account_id.clone();
            let take_id = account_id.clone();
            let legend = account.with_value(|a| (a.name.clone(), money(a.target, &a.currency), a.note.clone()));
            let can_take = !allow_negative && pots.with_value(|p| !p.is_empty());
            view! {
                <fieldset class="min-w-0 rounded-lg border border-border bg-card-muted p-4 sm:p-5">
                    <legend class="float-left mb-3 w-full text-sm">
                        <strong>{legend.0}</strong>" · "<span class="tabular-nums">{legend.1}</span>
                    </legend>
                    <div class="clear-both grid gap-4">
                        {legend.2.map(|note| view! { <small class=SMALL>{note}</small> })}
                        <For
                            each=lines
                            key=|d: &Draft| d.key.clone()
                            children=move |draft: Draft| {
                                view! {
                                    <SplitLine
                                        account=account
                                        index=index
                                        key_id=draft.key.clone()
                                        take=draft.take
                                        rows=rows
                                        pots=pots
                                        goals=goals
                                        categories=categories
                                        min_plan_month=min_plan_month
                                        allow_negative=allow_negative
                                    />
                                }
                            }
                        />
                        <div class="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
                            <div class=ACTIONS>
                                <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| {
                                    let remaining = rows.with_untracked(|r| r.rows.get(index).map(|row| row.remaining)).unwrap_or_default();
                                    let unit = account.with_value(|a| a.currency.minor_unit);
                                    let draft = Draft {
                                        key: new_id(),
                                        destination: "buffer".into(),
                                        amount: if remaining <= Decimal::ZERO { String::new() } else { fixed(remaining, unit) },
                                        ..Draft::default()
                                    };
                                    set_account(add_id.clone(), Box::new(move |mut l| { l.push(draft); l }));
                                })>"Add line"</Button>
                                {can_take.then(|| {
                                    let take_id = take_id.clone();
                                    view! {
                                        <Button size=Size::Sm variant=Variant::Secondary on_click=Callback::new(move |_| {
                                            let general = general.get_value();
                                            let pot = pots.with_value(|p| {
                                                p.iter()
                                                    .find(|pot| format!("pot:{}", pot.pot_id) == general)
                                                    .or_else(|| p.first())
                                                    .map(|pot| pot.pot_id.clone())
                                            });
                                            let Some(pot) = pot else { return };
                                            let draft = Draft {
                                                key: new_id(),
                                                destination: format!("pot:{pot}"),
                                                take: true,
                                                ..Draft::default()
                                            };
                                            set_account(take_id.clone(), Box::new(move |mut l| { l.push(draft); l }));
                                        })>"Take from pot"</Button>
                                    }
                                })}
                            </div>
                            {move || row().map(|row| {
                                let ok = row.remaining.is_zero() && row.valid;
                                let text = if row.valid {
                                    format!("{} left to assign", money(row.remaining, &row.account.currency))
                                } else if allow_negative {
                                    "Enter non-zero amounts".to_owned()
                                } else {
                                    "Enter positive amounts".to_owned()
                                };
                                view! {
                                    <strong class=cn(["text-sm tabular-nums", if ok { "" } else { "text-negative-fg" }])>{text}</strong>
                                }
                            })}
                        </div>
                    </div>
                </fieldset>
            }
        })
        .collect_view();
    let submit_label = StoredValue::new(submit_label);
    view! {
        <div class="grid gap-4">
            {suggestion_view}
            {fieldsets}
            <div>
                <Button
                    disabled=Signal::derive(move || disabled.get() || !rows.with(|r| r.complete))
                    on_click=Callback::new(submit)
                >
                    {submit_label.get_value()}
                </Button>
            </div>
        </div>
    }
}

/// One allocation line: destination, amount, and the extra fields a plan
/// needs.
#[component]
fn SplitLine(
    account: StoredValue<SplitTarget>,
    index: usize,
    key_id: String,
    take: bool,
    rows: Memo<Rows>,
    pots: StoredValue<Vec<PotView>>,
    goals: StoredValue<Vec<(String, String)>>,
    categories: StoredValue<Vec<String>>,
    min_plan_month: StoredValue<String>,
    allow_negative: bool,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let store = wizard.drafts;
    let account_id = StoredValue::new(account.with_value(|a| a.account_id.clone()));
    let key = StoredValue::new(key_id);
    let currency = account.with_value(|a| a.currency.clone());
    let draft = Memo::new(move |_| {
        store.with(|s| {
            s.accounts
                .get(&account_id.get_value())
                .and_then(|lines| lines.iter().find(|d| d.key == key.get_value()))
                .cloned()
                .unwrap_or_default()
        })
    });
    let update = move |change: Box<dyn FnOnce(&mut Draft)>| {
        store.update(|s| {
            if let Some(d) = s
                .accounts
                .get_mut(&account_id.get_value())
                .and_then(|lines| lines.iter_mut().find(|d| d.key == key.get_value()))
            {
                change(d);
            }
        });
    };
    let position = move || {
        rows.with(|r| {
            r.rows.get(index).and_then(|row| {
                row.lines
                    .iter()
                    .position(|d| d.key == key.get_value())
                    .map(|i| {
                        (
                            row.parsed[i].is_none(),
                            row.errors[i].clone(),
                            row.lines.len(),
                            row.account.target,
                        )
                    })
            })
        })
    };
    let invalid_amount = Signal::derive(move || position().is_some_and(|p| p.0));
    let error = move || position().and_then(|p| p.1);
    let can_remove = Signal::derive(move || {
        position()
            .is_some_and(|(_, _, count, target)| count > 1 || (!allow_negative && target.is_zero()))
    });
    let destination_options = {
        let currency = currency.clone();
        Signal::derive(move || {
            let mut list: Vec<(String, String)> = Vec::new();
            let pot_options = pots.with_value(|p| {
                p.iter()
                    .map(|pot| (format!("pot:{}", pot.pot_id), pot_label(pot, &currency)))
                    .collect::<Vec<_>>()
            });
            if take {
                list.extend(pot_options);
            } else {
                list.push(("stay".into(), "Stays on this account".into()));
                list.push(("buffer".into(), "Next month's buffer".into()));
                list.extend(pot_options);
                goals.with_value(|g| {
                    list.extend(
                        g.iter()
                            .map(|(id, name)| (format!("goal:{id}"), format!("Goal: {name}"))),
                    );
                });
                list.push(("plan".into(), "Plan a purchase…".into()));
            }
            let current = draft.with(|d| d.destination.clone());
            let mut list = options(list);
            if !list.iter().any(|(v, _, _)| *v == current) {
                list.insert(0, (current.clone(), current, false));
            }
            list
        })
    };
    let destination_id = field_id();
    let amount_id = field_id();
    let destination_label = if take {
        view! { <span class="text-positive-fg">"Take from pot"</span> }.into_any()
    } else {
        "Destination".into_any()
    };
    let amount_label = format!(
        "{} ({})",
        if take { "Take" } else { "Amount" },
        currency.code
    );
    let unit = currency.minor_unit;
    let is_plan = Memo::new(move |_| draft.with(|d| d.destination == "plan"));

    let plan_form = move || {
        let what = field_id();
        let due = field_id();
        let category = field_id();
        let shop = field_id();
        let min = min_plan_month.get_value();
        let min2 = min.clone();
        view! {
            <div class=cn([INLINE_FORM, "sm:col-span-3"]) role="group" aria-label="Planned purchase">
                <FieldBox label="What".into_any() id=what.clone() class=GROW>
                    <TextBox
                        id=what
                        value=Signal::derive(move || draft.with(|d| d.plan_name.clone()))
                        on_input=Callback::new(move |v: String| update(Box::new(move |d| d.plan_name = v)))
                        maxlength=128
                        placeholder="e.g. New laptop"
                        invalid=Signal::derive(move || draft.with(|d| d.plan_name.trim().is_empty()))
                    />
                </FieldBox>
                <FieldBox label="Due month".into_any() id=due.clone() class=GROW>
                    <TextBox
                        id=due
                        kind="month"
                        min=min.clone()
                        value=Signal::derive(move || draft.with(|d| d.plan_month.clone().unwrap_or_else(|| min2.clone())))
                        on_input=Callback::new(move |v: String| update(Box::new(move |d| d.plan_month = Some(v))))
                    />
                </FieldBox>
                <FieldBox label="Match by category".into_any() id=category.clone() class=GROW>
                    {move || {
                        let current = draft.with(|d| d.plan_category.clone());
                        let known = categories.get_value();
                        let mut list = known.clone();
                        if !current.is_empty() && !known.contains(&current) {
                            list.insert(0, current.clone());
                        }
                        if list.is_empty() {
                            view! {
                                <TextBox
                                    id=category.clone()
                                    value=Signal::derive(move || draft.with(|d| d.plan_category.clone()))
                                    on_input=Callback::new(move |v: String| update(Box::new(move |d| d.plan_category = v)))
                                    maxlength=128
                                    placeholder="e.g. Electronics"
                                />
                            }
                            .into_any()
                        } else {
                            let mut opts = vec![(String::new(), "Not set".to_owned(), false)];
                            opts.extend(list.into_iter().map(|n| (n.clone(), n, false)));
                            view! {
                                <Choice
                                    id=category.clone()
                                    value=Signal::derive(move || draft.with(|d| d.plan_category.clone()))
                                    options=Signal::stored(opts)
                                    on_change=Callback::new(move |v: String| update(Box::new(move |d| d.plan_category = v)))
                                />
                            }
                            .into_any()
                        }
                    }}
                </FieldBox>
                <FieldBox label="or shop name".into_any() id=shop.clone() class=GROW>
                    <TextBox
                        id=shop
                        value=Signal::derive(move || draft.with(|d| d.plan_shop.clone()))
                        on_input=Callback::new(move |v: String| update(Box::new(move |d| d.plan_shop = v)))
                        maxlength=128
                        placeholder="e.g. Coolblue"
                    />
                </FieldBox>
                <small class=cn([SMALL, "basis-full"])>
                    "Becomes a planned payment from this account; it does not count as this month's spending. The money carries into the due month, and a purchase in that month (from any account, up to 10% more) marks it paid."
                </small>
            </div>
        }
    };

    view! {
        <div class="grid items-end gap-3 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_auto]">
            <FieldBox label=destination_label id=destination_id.clone()>
                <Choice
                    id=destination_id
                    value=Signal::derive(move || draft.with(|d| d.destination.clone()))
                    options=destination_options
                    on_change=Callback::new(move |destination: String| {
                        let min = min_plan_month.get_value();
                        update(Box::new(move |d| {
                            if destination == "plan" && d.plan_month.is_none() {
                                d.plan_month = Some(min);
                            }
                            d.destination = destination;
                        }));
                    })
                />
            </FieldBox>
            <FieldBox label=amount_label.into_any() id=amount_id.clone()>
                <TextBox
                    id=amount_id
                    inputmode="decimal"
                    value=Signal::derive(move || draft.with(|d| d.amount.clone()))
                    invalid=invalid_amount
                    on_input=Callback::new(move |v: String| update(Box::new(move |d| d.amount = v)))
                    on_blur=Callback::new(move |_| {
                        let amount = draft.with_untracked(|d| d.amount.clone());
                        let normal = minor_amount(&amount, unit);
                        if normal != amount {
                            update(Box::new(move |d| d.amount = normal));
                        }
                    })
                />
            </FieldBox>
            <div>
                <Button size=Size::Sm variant=Variant::Ghost disabled=Signal::derive(move || !can_remove.get()) on_click=Callback::new(move |_| {
                    let k = key.get_value();
                    store.update(|s| {
                        if let Some(lines) = s.accounts.get_mut(&account_id.get_value()) {
                            lines.retain(|d| d.key != k);
                        }
                    });
                })>"Remove"</Button>
            </div>
            <Show when=move || is_plan.get()>{plan_form}</Show>
            {move || error().map(|e| view! { <small class=cn([WARN, "text-xs sm:col-span-3"])>{e}</small> })}
        </div>
    }
}
