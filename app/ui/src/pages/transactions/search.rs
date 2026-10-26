//! "All transactions": search with filters, totals, paging and an inline
//! category editor (personal-site `transaction-search.tsx`).

use std::str::FromStr;
use std::time::Duration;

use finance_types::report::review::{
    CategoryChange, CategorySaved, SearchDirection, SearchRow, TransactionSearchQuery,
    TransactionSearchResult,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{
    Choice, TextInput, clean_error, debounce, options, report_categories, storage_get, storage_set,
};
use crate::api::invoke;
use crate::ds::{
    CardTone, EmptyState, Eyebrow, Field, Heading, Icon, Lucide, Money, Size, Skeleton, TABLE,
    TABLE_CONTAINER, TCELL, TCELL_NUM, TH, TH_NUM, TROW, Variant, button_class, card_class, cn,
};
use crate::state;

const STORAGE_KEY: &str = "finance.transaction-search";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Filters {
    q: String,
    account: String,
    from: String,
    to: String,
    direction: String,
    min: String,
    max: String,
    #[serde(skip)]
    page: u32,
}

impl Filters {
    fn saved() -> Self {
        storage_get(STORAGE_KEY)
            .and_then(|s| js_sys::JSON::parse(&s).ok())
            .and_then(|v| serde_wasm_bindgen::from_value(v).ok())
            .unwrap_or_default()
    }

    fn query(&self) -> Result<TransactionSearchQuery, String> {
        let amount = |value: &str, key: &str| -> Result<Option<Decimal>, String> {
            if value.is_empty() {
                return Ok(None);
            }
            Decimal::from_str(value)
                .map(Some)
                .map_err(|_| format!("{key} must be a number"))
        };
        let some = |v: &str| (!v.is_empty()).then(|| v.to_owned());
        Ok(TransactionSearchQuery {
            q: self.q.clone(),
            account: some(&self.account),
            from: some(&self.from),
            to: some(&self.to),
            direction: match self.direction.as_str() {
                "in" => Some(SearchDirection::In),
                "out" => Some(SearchDirection::Out),
                "transfer" => Some(SearchDirection::Transfer),
                _ => None,
            },
            min: amount(&self.min, "min")?,
            max: amount(&self.max, "max")?,
            page: self.page.max(1),
            page_size: 50,
        })
    }

    fn filter_count(&self) -> usize {
        [
            &self.account,
            &self.from,
            &self.to,
            &self.direction,
            &self.min,
            &self.max,
        ]
        .iter()
        .filter(|v| !v.is_empty())
        .count()
    }
}

#[derive(Serialize)]
struct SearchArgs {
    query: TransactionSearchQuery,
}

#[derive(Serialize)]
struct ChangeArgs {
    change: CategoryChange,
}

fn title(row: &SearchRow) -> String {
    row.counterparty
        .clone()
        .or_else(|| row.description.clone())
        .unwrap_or_else(|| "Bank transaction".into())
}

fn sign(direction: SearchDirection) -> &'static str {
    match direction {
        SearchDirection::In => "+",
        SearchDirection::Out => "−",
        SearchDirection::Transfer => "",
    }
}

#[component]
pub fn TransactionSearch() -> impl IntoView {
    let app = state();
    let start = Filters::saved();
    let filters = RwSignal::new(Filters {
        page: 1,
        ..start.clone()
    });
    let query = RwSignal::new(start.q.clone());
    let min = RwSignal::new(start.min.clone());
    let max = RwSignal::new(start.max.clone());
    let show_filters = RwSignal::new(false);
    let result = RwSignal::new(None::<TransactionSearchResult>);
    let error = RwSignal::new(String::new());
    let loading = RwSignal::new(true);
    let retry = RwSignal::new(0_u32);
    let generation = StoredValue::new(0_u64);
    let categories = report_categories(&[]);

    debounce(query, Duration::from_millis(300), move |q: String| {
        let q = q.trim().to_owned();
        if filters.with_untracked(|f| f.q != q) {
            filters.update(|f| {
                f.q = q;
                f.page = 1;
            });
        }
    });
    let range = move |_: String| {
        let (low, high) = (
            min.get_untracked().trim().replace(',', "."),
            max.get_untracked().trim().replace(',', "."),
        );
        if filters.with_untracked(|f| f.min != low || f.max != high) {
            filters.update(|f| {
                f.min = low;
                f.max = high;
                f.page = 1;
            });
        }
    };
    debounce(min, Duration::from_millis(300), range);
    debounce(max, Duration::from_millis(300), range);

    Effect::new(move |_| {
        let current = filters.get();
        retry.track();
        app.revision.track();
        if let Some(json) = serde_wasm_bindgen::to_value(&current)
            .ok()
            .and_then(|v| js_sys::JSON::stringify(&v).ok())
            .and_then(|v| v.as_string())
        {
            storage_set(STORAGE_KEY, &json);
        }
        generation.update_value(|g| *g += 1);
        let mine = generation.get_value();
        loading.set(true);
        error.set(String::new());
        let query = match current.query() {
            Ok(q) => q,
            Err(e) => {
                error.set(e);
                loading.set(false);
                return;
            }
        };
        spawn_local(async move {
            let found =
                invoke::<TransactionSearchResult>("search_transactions", &SearchArgs { query })
                    .await;
            if generation.get_value() != mine {
                return;
            }
            match found {
                Ok(page) if page.items.is_empty() && page.total_count > 0 && page.page > 1 => {
                    filters.update(|f| f.page = 1);
                    return;
                }
                Ok(page) => result.set(Some(page)),
                Err(e) => {
                    let e = clean_error(e);
                    error.set(if e.is_empty() {
                        "Could not search transactions.".into()
                    } else {
                        e
                    });
                }
            }
            loading.set(false);
        });
    });

    let set = move |key: &'static str, value: String| {
        filters.update(|f| {
            match key {
                "account" => f.account = value,
                "from" => f.from = value,
                "to" => f.to = value,
                "direction" => f.direction = value,
                _ => {}
            }
            f.page = 1;
        });
    };
    let clear = move || {
        query.set(String::new());
        min.set(String::new());
        max.set(String::new());
        filters.set(Filters {
            page: 1,
            ..Filters::default()
        });
    };
    let accounts = Signal::derive(move || {
        let mut list = options([("", "All accounts")]);
        if let Some(r) = result.get() {
            list.extend(options(
                r.accounts.into_iter().map(|a| (a.account_id, a.name)),
            ));
        }
        list
    });
    let filter_attr = move |key: &'static str| {
        Signal::derive(move || {
            filters.with(|f| match key {
                "account" => f.account.clone(),
                "from" => f.from.clone(),
                "to" => f.to.clone(),
                _ => f.direction.clone(),
            })
        })
    };
    let from = RwSignal::new(start.from);
    let to = RwSignal::new(start.to);
    Effect::new(move |_| {
        let (f, t) = filters.with(|f| (f.from.clone(), f.to.clone()));
        if from.get_untracked() != f {
            from.set(f);
        }
        if to.get_untracked() != t {
            to.set(t);
        }
    });

    view! {
        <section class=cn([card_class(CardTone::Default), "my-5 grid min-w-0 gap-4"]) aria-label="All transactions">
            <div>
                <Eyebrow level=Heading::P>"SEARCH"</Eyebrow>
                <h2 class="mt-1 text-xl font-semibold">"All transactions"</h2>
            </div>
            <div class="flex flex-wrap items-end gap-2">
                <Field label="Search" id="tx-search-q" class="min-w-0 flex-1 basis-64">
                    <TextInput
                        id="tx-search-q"
                        kind="search"
                        value=query
                        maxlength=128
                        placeholder="Name, description, category or account"
                    />
                </Field>
                <button
                    type="button"
                    class=cn([button_class(Variant::Secondary, Size::Md).as_str(), "sm:hidden"])
                    aria-expanded=move || show_filters.get().to_string()
                    on:click=move |_| show_filters.update(|o| *o = !*o)
                >
                    <Lucide icon=Icon::SlidersHorizontal size=15 />
                    {move || {
                        let n = filters.with(Filters::filter_count);
                        if n > 0 { format!("Filters ({n})") } else { "Filters".into() }
                    }}
                </button>
            </div>
            <div class=move || cn([
                "flex-wrap items-end gap-2",
                if show_filters.get() { "flex" } else { "hidden sm:flex" },
            ])>
                <Field label="Account" id="tx-search-account" class="min-w-0 basis-40">
                    <Choice
                        id="tx-search-account"
                        value=filter_attr("account")
                        options=accounts
                        on_change=Callback::new(move |v| set("account", v))
                    />
                </Field>
                <Field label="From" id="tx-search-from" class="basis-36">
                    <TextInput
                        id="tx-search-from"
                        kind="date"
                        value=from
                        on_input=Callback::new(move |v| set("from", v))
                    />
                </Field>
                <Field label="To" id="tx-search-to" class="basis-36">
                    <TextInput
                        id="tx-search-to"
                        kind="date"
                        value=to
                        on_input=Callback::new(move |v| set("to", v))
                    />
                </Field>
                <Field label="Direction" id="tx-search-direction" class="basis-32">
                    <Choice
                        id="tx-search-direction"
                        value=filter_attr("direction")
                        options=options([("", "All"), ("in", "In"), ("out", "Out"), ("transfer", "Transfer")])
                        on_change=Callback::new(move |v| set("direction", v))
                    />
                </Field>
                <Field label="Min amount" id="tx-search-min" class="basis-28">
                    <TextInput id="tx-search-min" inputmode="decimal" value=min />
                </Field>
                <Field label="Max amount" id="tx-search-max" class="basis-28">
                    <TextInput id="tx-search-max" inputmode="decimal" value=max />
                </Field>
                <button type="button" class=button_class(Variant::Ghost, Size::Md) on:click=move |_| clear()>
                    "Clear"
                </button>
            </div>
            {move || {
                let r = result.get()?;
                if !error.get().is_empty() {
                    return None;
                }
                let n = r.total_count;
                Some(view! {
                    <div class="grid gap-1 text-sm" aria-live="polite">
                        {r.totals.is_empty().then(|| view! { <p>{format!("{n} transactions")}</p> })}
                        {r.totals
                            .into_iter()
                            .map(|t| {
                                view! {
                                    <p>
                                        {format!("{n} transactions · In ")}
                                        <Money value=t.inflow unit=t.currency.clone() neutral=true />
                                        " · Out "
                                        <Money value=t.outflow unit=t.currency.clone() neutral=true />
                                        " · Net "
                                        <Money value=t.net unit=t.currency />
                                    </p>
                                }
                            })
                            .collect_view()}
                    </div>
                })
            }}
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! {
                    <div role="alert" class="flex flex-wrap items-center gap-3 text-sm text-negative-fg">
                        {e}
                        <button
                            type="button"
                            class=button_class(Variant::Secondary, Size::Sm)
                            on:click=move |_| retry.update(|n| *n += 1)
                        >
                            "Retry"
                        </button>
                    </div>
                })
            }}
            {move || {
                let busy = loading.get();
                match result.get() {
                    None if busy => view! {
                        <div class="grid gap-2" role="status" aria-label="Loading transactions">
                            {(0..4).map(|_| view! { <Skeleton class="h-10" /> }).collect_view()}
                        </div>
                    }
                    .into_any(),
                    None => ().into_any(),
                    Some(r) if r.items.is_empty() => view! {
                        <EmptyState
                            title="No transactions match"
                            action=Box::new(move || view! {
                                <button type="button" class=button_class(Variant::Secondary, Size::Md) on:click=move |_| clear()>
                                    "Clear filters"
                                </button>
                            }.into_any())
                        />
                    }
                    .into_any(),
                    Some(r) => {
                        let pages = r.total_count.div_ceil(r.page_size.max(1) as usize).max(1) as u32;
                        let page = r.page;
                        let rows = r.items;
                        view! {
                            <div
                                tabindex="0"
                                role="region"
                                aria-label="Transactions"
                                class=cn([TABLE_CONTAINER, "hidden sm:block", if busy { "opacity-60" } else { "" }])
                            >
                                <table data-density="md" class=TABLE>
                                    <thead class="text-left">
                                        <tr class=TROW>
                                            <th class=TH>"Date"</th>
                                            <th class=TH>"Counterparty"</th>
                                            <th class=TH>"Account"</th>
                                            <th class=TH>"Category"</th>
                                            <th class=TH_NUM>"Amount"</th>
                                            <th class=TH><span class="sr-only">"Actions"</span></th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows
                                            .clone()
                                            .into_iter()
                                            .map(|row| {
                                                let detail = row.counterparty.is_some().then(|| row.description.clone()).flatten();
                                                let amount_class = cn([TCELL_NUM, if row.direction == SearchDirection::In { "text-positive-fg" } else { "" }]);
                                                view! {
                                                    <tr class=TROW>
                                                        <td class=cn([TCELL, "whitespace-nowrap"])>{row.occurred_on.clone()}</td>
                                                        <td class=TCELL>
                                                            <span class="font-medium">{title(&row)}</span>
                                                            {detail.map(|d| view! { <small class="block text-xs text-fg-muted">{d}</small> })}
                                                        </td>
                                                        <td class=cn([TCELL, "text-fg-muted"])>{row.account_name.clone().unwrap_or_else(|| "—".into())}</td>
                                                        <td class=TCELL><CategoryCell row=row.clone() categories=categories /></td>
                                                        <td class=amount_class>
                                                            {sign(row.direction)}
                                                            <Money value=row.amount.amount unit=row.amount.currency.clone() neutral=true />
                                                        </td>
                                                        <td class=TCELL></td>
                                                    </tr>
                                                }
                                            })
                                            .collect_view()}
                                    </tbody>
                                </table>
                            </div>
                            <ul class=cn(["grid gap-0 sm:hidden", if busy { "opacity-60" } else { "" }]) aria-label="Transactions">
                                {rows
                                    .into_iter()
                                    .map(|row| {
                                        let detail = row
                                            .counterparty
                                            .is_some()
                                            .then(|| row.description.clone())
                                            .flatten()
                                            .map(|d| format!(" · {d}"))
                                            .unwrap_or_default();
                                        let meta = format!(
                                            "{} · {}{detail}",
                                            row.occurred_on,
                                            row.account_name.clone().unwrap_or_else(|| "—".into())
                                        );
                                        view! {
                                            <li class="grid gap-2 border-t border-line py-3">
                                                <div class="flex justify-between gap-3">
                                                    <span class="min-w-0 break-words">
                                                        <strong class="font-semibold">{title(&row)}</strong>
                                                        <small class="block text-xs text-fg-muted">{meta}</small>
                                                    </span>
                                                    <span class=cn(["shrink-0 text-right", if row.direction == SearchDirection::In { "text-positive-fg" } else { "" }])>
                                                        {sign(row.direction)}
                                                        <Money value=row.amount.amount unit=row.amount.currency.clone() neutral=true />
                                                    </span>
                                                </div>
                                                <div class="flex items-center justify-between gap-3">
                                                    <CategoryCell row=row.clone() categories=categories />
                                                </div>
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                            {(pages > 1).then(|| view! {
                                <nav class="flex items-center justify-between gap-2" aria-label="Pages">
                                    <button
                                        type="button"
                                        class=button_class(Variant::Secondary, Size::Sm)
                                        disabled={page <= 1 || busy}
                                        on:click=move |_| filters.update(|f| f.page = page - 1)
                                    >
                                        "Previous"
                                    </button>
                                    <span class="text-sm text-fg-muted">{format!("Page {page} of {pages}")}</span>
                                    <button
                                        type="button"
                                        class=button_class(Variant::Secondary, Size::Sm)
                                        disabled={page >= pages || busy}
                                        on:click=move |_| filters.update(|f| f.page = page + 1)
                                    >
                                        "Next"
                                    </button>
                                </nav>
                            })}
                        }
                        .into_any()
                    }
                }
            }}
        </section>
    }
}

/// The inline category editor of one row (transfers show their category).
#[component]
fn CategoryCell(row: SearchRow, categories: Memo<Vec<String>>) -> impl IntoView {
    let app = state();
    if row.direction == SearchDirection::Transfer {
        let text = row.category.unwrap_or_else(|| "Transfer".into());
        return view! { <span class="text-fg-muted">{text}</span> }.into_any();
    }
    let busy = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let current = row.category.clone();
    let shown = RwSignal::new(current.clone().unwrap_or_default());
    let list = {
        let current = current.clone();
        Signal::derive(move || {
            let mut names: Vec<String> = current.iter().cloned().collect();
            for name in categories.get() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            let mut list = options([(
                "",
                if current.is_some() {
                    "Choose category"
                } else {
                    "Uncategorised"
                },
            )]);
            list.extend(names.into_iter().map(|n| (n.clone(), n, false)));
            list
        })
    };
    let id = row.transaction_id.clone();
    let save = Callback::new(move |category: String| {
        if category.is_empty() || Some(&category) == current.as_ref() || busy.get_untracked() {
            return;
        }
        shown.set(category.clone());
        busy.set(true);
        error.set(String::new());
        let change = CategoryChange {
            transaction_id: id.clone(),
            expected_category: current.clone(),
            category,
        };
        let previous = current.clone().unwrap_or_default();
        spawn_local(async move {
            match invoke::<CategorySaved>("set_transaction_category", &ChangeArgs { change }).await
            {
                Ok(_) => app.refresh(),
                Err(e) => {
                    let e = clean_error(e);
                    error.set(if e == "This payment changed. Refresh and try again." {
                        "This transaction changed. Refresh before saving.".into()
                    } else if e.is_empty() {
                        "Could not save the category.".into()
                    } else {
                        e
                    });
                    shown.set(previous);
                }
            }
            busy.set(false);
        });
    });
    let label = format!("Category for {}", title(&row));
    view! {
        <span class="grid gap-1">
            <Choice
                label=label
                value=shown
                options=list
                disabled=busy
                on_change=save
            />
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! { <small role="alert" class="text-xs text-negative-fg">{e}</small> })
            }}
        </span>
    }
    .into_any()
}
