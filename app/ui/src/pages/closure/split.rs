//! The split editor and its lines (personal-site
//! `components/month-closure/split-editor.tsx` and `split-line.tsx`).

use std::collections::{BTreeMap, HashMap};

use finance_types::report::Currency;
use finance_types::report::closure::{
    AllocationLine, LineKind, SavingsSuggestion, TransferStatus, WishOption,
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
    /// As typed.
    pub amount: String,
    pub plan_name: String,
    /// YYYY-MM; saved as the first day of that month.
    pub plan_month: Option<String>,
    pub plan_category: String,
    pub plan_shop: String,
    pub plan_id: Option<String>,
}

/// The drafts of one split, kept by the wizard so a reload of the month
/// does not lose what was typed.
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

/// Names for destinations the list no longer offers: `(pot_id, name)` of
/// old pots and `(wish_id, label)` of every wish.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyNames {
    pub pots: Vec<(String, String)>,
    pub wishes: Vec<(String, String)>,
}

/// The label of a destination the list no longer offers, `None` for one it
/// does: `Pot (retired): <name>`, or what became of a wish that cannot take
/// a line (`Wish (bought): <name>` …).
pub fn legacy_label(
    destination: &str,
    wishes: &[WishOption],
    legacy: &LegacyNames,
) -> Option<String> {
    let find = |list: &[(String, String)], id: &str| {
        list.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone())
    };
    if let Some(pot) = destination.strip_prefix("pot:") {
        let name = find(&legacy.pots, pot).unwrap_or_else(|| pot.to_owned());
        return Some(format!("Pot (retired): {name}"));
    }
    let wish = destination.strip_prefix("wish:")?;
    if wishes.iter().any(|w| w.wish_id == wish) {
        return None;
    }
    Some(find(&legacy.wishes, wish).unwrap_or_else(|| format!("Wish (no longer planned): {wish}")))
}

/// `(value, label)` of every destination a line can pick, in order: Stays on
/// this account, Next month's buffer, each wish, each goal, Plan a purchase….
/// A `current` value the list does not offer (an old pot, a bought wish)
/// comes first with its legacy label.
pub fn destination_options(
    wishes: &[WishOption],
    goals: &[(String, String)],
    current: &str,
    legacy: &LegacyNames,
) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = vec![
        ("stay".into(), "Stays on this account".into()),
        ("buffer".into(), "Next month's buffer".into()),
    ];
    list.extend(
        wishes
            .iter()
            .map(|w| (format!("wish:{}", w.wish_id), w.label.clone())),
    );
    list.extend(
        goals
            .iter()
            .map(|(id, name)| (format!("goal:{id}"), format!("Goal: {name}"))),
    );
    list.push(("plan".into(), "Plan a purchase…".into()));
    if !list.iter().any(|(v, _)| v == current) {
        let label = legacy_label(current, wishes, legacy).unwrap_or_else(|| current.to_owned());
        list.insert(0, (current.to_owned(), label));
    }
    list
}

/// The signed amount a draft stands for, or `None` when it is not a valid
/// amount for its line. A legacy line keeps whatever it was saved with.
fn parse(draft: &Draft, minor_unit: u8, allow_negative: bool, legacy: bool) -> Option<Decimal> {
    let value = parse_amount(draft.amount.trim())?;
    if decimal_places(value) > u32::from(minor_unit) {
        return None;
    }
    if draft.destination == "plan" || (draft.destination.starts_with("wish:") && !legacy) {
        return (value > Decimal::ZERO).then_some(value);
    }
    let bad = if allow_negative || legacy {
        value.is_zero()
    } else {
        value <= Decimal::ZERO
    };
    (!bad).then_some(value)
}

fn draft_from(line: &AllocationLine) -> Draft {
    let amount = minor_amount(&line.amount.to_string(), line.currency.minor_unit);
    let plan = line.destination == "plan";
    Draft {
        key: line.line_id.clone(),
        destination: line.destination.clone(),
        amount,
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
    wishes: Vec<WishOption>,
    legacy: LegacyNames,
    allow_negative: bool,
    min_plan_month: String,
}

/// The wishes a line from an account in `currency` can save toward.
fn wishes_in(wishes: &[WishOption], currency: &Currency) -> Vec<WishOption> {
    wishes
        .iter()
        .filter(|w| w.currency == *currency)
        .cloned()
        .collect()
}

fn compute(setup: &Setup, drafts: &DraftStore) -> Rows {
    // What each wish can still take, less the lines above this one.
    let mut room: HashMap<&str, Decimal> = setup
        .wishes
        .iter()
        .map(|w| (w.wish_id.as_str(), w.room))
        .collect();
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
            let offered = wishes_in(&setup.wishes, &account.currency);
            let legacy: Vec<bool> = lines
                .iter()
                .map(|l| legacy_label(&l.destination, &offered, &setup.legacy).is_some())
                .collect();
            let parsed: Vec<Option<Decimal>> = lines
                .iter()
                .zip(&legacy)
                .map(|(l, old)| parse(l, unit, setup.allow_negative, *old))
                .collect();
            let errors: Vec<Option<String>> = lines
                .iter()
                .zip(&parsed)
                .zip(&legacy)
                .map(|((line, value), old)| {
                    if *old {
                        let elsewhere = line
                            .destination
                            .strip_prefix("wish:")
                            .and_then(|id| setup.wishes.iter().find(|w| w.wish_id == id));
                        return Some(if line.destination.starts_with("pot:") {
                            "Pots are retired: remove this line.".into()
                        } else if let Some(wish) = elsewhere {
                            format!(
                                "The wish saves in {}, not {}: remove this line.",
                                wish.currency.code, account.currency.code
                            )
                        } else {
                            "This wish takes no more money: remove this line.".into()
                        });
                    }
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
                    if let (Some(wish), Some(value)) =
                        (line.destination.strip_prefix("wish:"), value)
                        && let Some(left) = room.get_mut(wish)
                    {
                        let before = *left;
                        *left -= *value;
                        if *left < Decimal::ZERO {
                            return Some(format!(
                                "More than the wish still needs ({} left).",
                                money(before.max(Decimal::ZERO), &account.currency)
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
/// to its target before the owner can submit. Split mode needs positive
/// amounts; delta mode (resettle) allows negatives, except toward a wish.
/// An old line to a pot or to a wish that takes no more money shows read
/// only and must be removed.
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
    /// Planned wishes a line can save toward.
    #[prop(optional)]
    wishes: Vec<WishOption>,
    /// Names for old lines' destinations.
    #[prop(optional)]
    legacy: LegacyNames,
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
                // only lists old lines to remove.
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
        wishes: wishes.clone(),
        legacy: legacy.clone(),
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
                open=suggestion_open
            />
        })
    };

    let goals = StoredValue::new(goals);
    let legacy = StoredValue::new(legacy);
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
            let legend = account.with_value(|a| (a.name.clone(), money(a.target, &a.currency), a.note.clone()));
            let offered = StoredValue::new(account.with_value(|a| wishes_in(&wishes, &a.currency)));
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
                                        rows=rows
                                        wishes=offered
                                        legacy=legacy
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
/// needs. An old line (a pot, a wish that takes no more money) is read only.
#[component]
fn SplitLine(
    account: StoredValue<SplitTarget>,
    index: usize,
    key_id: String,
    rows: Memo<Rows>,
    wishes: StoredValue<Vec<WishOption>>,
    legacy: StoredValue<LegacyNames>,
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
    let old = Memo::new(move |_| {
        draft.with(|d| {
            wishes
                .with_value(|w| legacy.with_value(|l| legacy_label(&d.destination, w, l).is_some()))
        })
    });
    let can_remove = Signal::derive(move || {
        old.get()
            || position().is_some_and(|(_, _, count, target)| {
                count > 1 || (!allow_negative && target.is_zero())
            })
    });
    let destination_options = Signal::derive(move || {
        let current = draft.with(|d| d.destination.clone());
        let list = wishes.with_value(|w| {
            goals.with_value(|g| legacy.with_value(|l| destination_options(w, g, &current, l)))
        });
        options(list)
    });
    let destination_id = field_id();
    let amount_id = field_id();
    let amount_label = format!("Amount ({})", currency.code);
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
            <FieldBox label="Destination".into_any() id=destination_id.clone()>
                <Choice
                    id=destination_id
                    value=Signal::derive(move || draft.with(|d| d.destination.clone()))
                    options=destination_options
                    disabled=old
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
                    readonly=old
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wish(id: &str, label: &str) -> WishOption {
        WishOption {
            wish_id: id.into(),
            label: label.into(),
            currency: Currency::new("EUR"),
            room: Decimal::ONE_HUNDRED,
        }
    }

    fn pair(a: &str, b: &str) -> (String, String) {
        (a.into(), b.into())
    }

    fn legacy() -> LegacyNames {
        LegacyNames {
            pots: vec![pair("general", "General savings")],
            wishes: vec![pair("tv", "Wish (bought): TV")],
        }
    }

    fn values(list: &[(String, String)]) -> Vec<&str> {
        list.iter().map(|(v, _)| v.as_str()).collect()
    }

    fn account(id: &str, target: &str) -> SplitTarget {
        SplitTarget {
            account_id: id.into(),
            name: id.into(),
            currency: Currency::new("EUR"),
            target: target.parse().unwrap(),
            note: None,
        }
    }

    fn draft(destination: &str, amount: &str) -> Draft {
        Draft {
            key: new_key(),
            destination: destination.into(),
            amount: amount.into(),
            ..Draft::default()
        }
    }

    fn new_key() -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!("k{}", NEXT.fetch_add(1, Ordering::Relaxed))
    }

    fn rows(
        accounts: Vec<SplitTarget>,
        wishes: Vec<WishOption>,
        allow_negative: bool,
        lines: &[(&str, Vec<Draft>)],
    ) -> Rows {
        let setup = Setup {
            accounts,
            wishes,
            legacy: legacy(),
            allow_negative,
            min_plan_month: "2026-11".into(),
        };
        let drafts = DraftStore {
            key: "k".into(),
            accounts: lines
                .iter()
                .map(|(id, l)| ((*id).to_owned(), l.clone()))
                .collect(),
        };
        compute(&setup, &drafts)
    }

    #[test]
    fn a_legacy_line_blocks_the_split_until_removed() {
        let rows = rows(
            vec![account("a", "50")],
            Vec::new(),
            false,
            &[(
                "a",
                vec![draft("pot:general", "-20.00"), draft("stay", "70.00")],
            )],
        );
        assert!(!rows.complete);
        let row = &rows.rows[0];
        assert!(row.valid, "the old take keeps its negative amount");
        assert!(row.remaining.is_zero());
        assert_eq!(
            row.errors[0].as_deref(),
            Some("Pots are retired: remove this line.")
        );
    }

    #[test]
    fn wish_room_is_shared_across_lines_and_accounts() {
        let bike = wish("bike", "Wish: Bike"); // room 100
        let ok = rows(
            vec![account("a", "60"), account("b", "40")],
            vec![bike.clone()],
            false,
            &[
                ("a", vec![draft("wish:bike", "60.00")]),
                ("b", vec![draft("wish:bike", "40.00")]),
            ],
        );
        assert!(ok.complete);
        let over = rows(
            vec![account("a", "70"), account("b", "40")],
            vec![bike],
            false,
            &[
                (
                    "a",
                    vec![draft("wish:bike", "30.00"), draft("wish:bike", "40.00")],
                ),
                ("b", vec![draft("wish:bike", "40.00")]),
            ],
        );
        assert!(!over.complete);
        assert_eq!(over.rows[0].errors, [None, None]);
        assert_eq!(
            over.rows[1].errors[0].as_deref(),
            Some("More than the wish still needs (€30.00 left).")
        );
    }

    #[test]
    fn a_wish_line_stays_positive_in_delta_mode() {
        let rows = rows(
            vec![account("a", "-10")],
            vec![wish("bike", "Wish: Bike")],
            true,
            &[("a", vec![draft("wish:bike", "-10.00")])],
        );
        assert_eq!(rows.rows[0].parsed, [None]);
        assert!(!rows.complete);
        let buffer = super::parse(&draft("buffer", "-10.00"), 2, true, false);
        assert_eq!(buffer, Some("-10.00".parse().unwrap()));
    }

    #[test]
    fn a_wish_in_another_currency_says_so() {
        let mut usd = wish("trip", "Wish: Trip");
        usd.currency = Currency::new("USD");
        let rows = rows(
            vec![account("a", "10")],
            vec![usd],
            false,
            &[("a", vec![draft("wish:trip", "10.00")])],
        );
        assert_eq!(
            rows.rows[0].errors[0].as_deref(),
            Some("The wish saves in USD, not EUR: remove this line.")
        );
    }

    #[test]
    fn destinations_list_wishes_before_goals() {
        let wishes = [
            wish("trip", "Wish: Trip rest"),
            wish("bike", "Wish: Bike (€0.00 of €900.00)"),
        ];
        let goals = [pair("house", "House")];
        let list = destination_options(&wishes, &goals, "stay", &legacy());
        assert_eq!(
            list,
            [
                pair("stay", "Stays on this account"),
                pair("buffer", "Next month's buffer"),
                pair("wish:trip", "Wish: Trip rest"),
                pair("wish:bike", "Wish: Bike (€0.00 of €900.00)"),
                pair("goal:house", "Goal: House"),
                pair("plan", "Plan a purchase…"),
            ]
        );
    }

    #[test]
    fn a_legacy_pot_stays_listed_once_with_its_retired_label() {
        let list = destination_options(&[], &[], "pot:general", &legacy());
        assert_eq!(
            list[0],
            pair("pot:general", "Pot (retired): General savings")
        );
        assert_eq!(
            values(&list)
                .iter()
                .filter(|v| v.starts_with("pot:"))
                .count(),
            1
        );
        let unknown = destination_options(&[], &[], "pot:gone", &legacy());
        assert_eq!(unknown[0], pair("pot:gone", "Pot (retired): gone"));
    }

    #[test]
    fn a_wish_no_longer_offered_keeps_its_line_with_what_became_of_it() {
        let wishes = [wish("bike", "Wish: Bike (€0.00 of €900.00)")];
        let bought = destination_options(&wishes, &[], "wish:tv", &legacy());
        assert_eq!(bought[0], pair("wish:tv", "Wish (bought): TV"));
        let unknown = destination_options(&wishes, &[], "wish:x", &legacy());
        assert_eq!(unknown[0], pair("wish:x", "Wish (no longer planned): x"));
        assert_eq!(
            values(&unknown).iter().filter(|v| **v == "wish:x").count(),
            1
        );
        // An offered wish is not repeated.
        let offered = destination_options(&wishes, &[], "wish:bike", &legacy());
        assert_eq!(
            values(&offered)
                .iter()
                .filter(|v| **v == "wish:bike")
                .count(),
            1
        );
        assert_eq!(legacy_label("wish:bike", &wishes, &legacy()), None);
        assert_eq!(legacy_label("goal:house", &wishes, &legacy()), None);
    }
}
