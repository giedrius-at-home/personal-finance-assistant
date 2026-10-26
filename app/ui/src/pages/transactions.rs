use finance_types::{TxKind, TxPage, TxRow};
use leptos::ev;
use leptos::html::Input;
use leptos::prelude::*;
use serde::Serialize;
use wasm_bindgen::JsCast;

use crate::api::load;
use crate::widgets::{Amount, Problem, Skeleton, amounts};
use crate::{CategoryFilter, Filters, Period, format, icons, state};

const PAGE: u32 = 100;

#[derive(Serialize)]
struct Args {
    query: finance_types::TxQuery,
}

#[component]
pub fn Transactions() -> impl IntoView {
    let app = state();
    let limit = RwSignal::new(PAGE);
    let selected = RwSignal::new(None::<TxRow>);

    // New filters start from the first page with nothing selected.
    Effect::new(move |prev: Option<Filters>| {
        let f = app.filters.get();
        if prev.is_some_and(|p| p != f) {
            limit.set(PAGE);
            selected.set(None);
        }
        f
    });
    let page = load::<TxPage, _>("transactions", move || Args {
        query: app.filters.get().query(limit.get()),
    });
    let rows = Memo::new(move |_| {
        page.data
            .get()
            .and_then(Result::ok)
            .map(|p| p.rows)
            .unwrap_or_default()
    });

    // ↑/↓ (or j/k) move through rows, Esc closes the details.
    let handle = window_event_listener(ev::keydown, move |e| {
        let typing = document()
            .active_element()
            .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "SELECT" | "TEXTAREA"));
        let key = e.key();
        if key == "Escape" {
            if typing {
                return;
            }
            selected.set(None);
            return;
        }
        if typing || e.meta_key() || e.ctrl_key() {
            return;
        }
        let step: isize = match key.as_str() {
            "ArrowDown" | "j" => 1,
            "ArrowUp" | "k" => -1,
            _ => return,
        };
        e.prevent_default();
        let list = rows.get_untracked();
        if list.is_empty() {
            return;
        }
        let current = selected.with_untracked(|s| {
            s.as_ref()
                .and_then(|r| list.iter().position(|x| x.id == r.id))
        });
        let next = match current {
            Some(i) => i.saturating_add_signed(step).min(list.len() - 1),
            None => 0,
        };
        let id = list[next].id;
        selected.set(Some(list[next].clone()));
        if let Some(el) = document().get_element_by_id(&format!("tx-{id}")) {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
            // Keep keyboard focus on the selected row so the ring follows it.
            if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
                let _ = el.focus();
            }
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="tx-layout" class:with-detail=move || selected.with(Option::is_some)>
            <div class="tx-main">
                <Toolbar />
                {move || match page.data.get() {
                    None => view! { <Skeleton rows=10 /> }.into_any(),
                    Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
                    Some(Ok(p)) => {
                        let shown = p.rows.len() as u64;
                        let total = p.total;
                        view! {
                            <div class="tx-summary" class:loading=move || page.loading.get()>
                                <span>{crate::pages::overview::plural(total, "transaction")}</span>
                                {(!p.sums.is_empty())
                                    .then(|| view! { <span class="muted">" · net "</span> {amounts(&p.sums, true)} })}
                            </div>
                            {if p.rows.is_empty() {
                                view! {
                                    <div class="empty card">
                                        <h2>"Nothing matches"</h2>
                                        <p class="muted">"Try fewer words or a wider period."</p>
                                        <button class="button" on:click=move |_| app.filters.set(Filters::default())>
                                            "Clear filters"
                                        </button>
                                    </div>
                                }
                                    .into_any()
                            } else {
                                view! { <Table rows=p.rows selected=selected /> }.into_any()
                            }}
                            {(shown < total)
                                .then(|| {
                                    view! {
                                        <div class="more">
                                            <button
                                                class="button"
                                                disabled=move || page.loading.get()
                                                on:click=move |_| limit.update(|l| *l += PAGE)
                                            >
                                                {format!("Show {} more", (total - shown).min(u64::from(PAGE)))}
                                            </button>
                                            <span class="muted small">
                                                {format!("{} of {}", format::count(shown), format::count(total))}
                                            </span>
                                        </div>
                                    }
                                })}
                        }
                            .into_any()
                    }
                }}
            </div>
            {move || selected.get().map(|row| view! { <Detail row=row selected=selected /> })}
        </div>
    }
}

#[component]
fn Toolbar() -> impl IntoView {
    let app = state();
    let input = NodeRef::<Input>::new();
    // The box updates instantly; the query follows after a short pause.
    let draft = RwSignal::new(app.filters.get_untracked().text);
    let typed = StoredValue::new(0_u64);
    Effect::new(move |_| {
        let text = app.filters.with(|f| f.text.clone());
        if text != draft.get_untracked() {
            draft.set(text);
        }
    });
    // Take focus only when asked (⌘F), not on every visit.
    Effect::new(move |_| {
        let asked = app.focus_search.get() > 0;
        if let Some(el) = input.get()
            && asked
        {
            let _ = el.focus();
            el.select();
        }
    });
    let on_input = move |e: ev::Event| {
        let value = event_target_value(&e);
        draft.set(value.clone());
        typed.update_value(|n| *n += 1);
        let mine = typed.get_value();
        set_timeout(
            move || {
                if typed.get_value() == mine {
                    app.filters.update(|f| f.text = value);
                }
            },
            std::time::Duration::from_millis(220),
        );
    };

    let facets = move || app.facets.data.get().and_then(Result::ok);
    let set = move |change: Box<dyn FnOnce(&mut Filters)>| app.filters.update(|f| change(f));

    view! {
        <div class="toolbar">
            <label class="search">
                {icons::search()}
                <input
                    node_ref=input
                    type="search"
                    placeholder="Search description, shop, category, note"
                    aria-label="Search transactions"
                    prop:value=move || draft.get()
                    on:input=on_input
                    on:keydown=move |e: ev::KeyboardEvent| {
                        if e.key() == "Escape" {
                            draft.set(String::new());
                            app.filters.update(|f| f.text.clear());
                        }
                    }
                />
                <kbd>"⌘F"</kbd>
            </label>

            <div class="segmented" role="group" aria-label="Type">
                {[None, Some(TxKind::Withdrawal), Some(TxKind::Deposit), Some(TxKind::Transfer)]
                    .into_iter()
                    .map(|kind| {
                        view! {
                            <button
                                class:active=move || app.filters.with(|f| f.kind == kind)
                                aria-pressed=move || app.filters.with(|f| f.kind == kind).to_string()
                                on:click=move |_| set(Box::new(move |f| f.kind = kind))
                            >
                                {kind.map(TxKind::label).unwrap_or("All")}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>

            <select
                aria-label="Account"
                on:change=move |e| {
                    let id = event_target_value(&e).parse().ok();
                    set(Box::new(move |f| f.account = id));
                }
                prop:value=move || app.filters.with(|f| f.account.map(|a| a.to_string()).unwrap_or_default())
            >
                <option value="">"All accounts"</option>
                {move || {
                    facets()
                        .map(|fa| {
                            fa.accounts
                                .into_iter()
                                .map(|a| view! { <option value=a.id.to_string()>{a.name}</option> })
                                .collect_view()
                        })
                }}
            </select>

            <select
                aria-label="Category"
                on:change=move |e| {
                    let value = event_target_value(&e);
                    let category = match value.as_str() {
                        "" => CategoryFilter::Any,
                        "none" => CategoryFilter::Uncategorised,
                        id => id.parse().map(CategoryFilter::Id).unwrap_or_default(),
                    };
                    set(Box::new(move |f| f.category = category));
                }
                prop:value=move || {
                    app.filters
                        .with(|f| match f.category {
                            CategoryFilter::Any => String::new(),
                            CategoryFilter::Uncategorised => "none".into(),
                            CategoryFilter::Id(id) => id.to_string(),
                        })
                }
            >
                <option value="">"All categories"</option>
                <option value="none">"Uncategorised spending"</option>
                {move || {
                    facets()
                        .map(|fa| {
                            fa.categories
                                .into_iter()
                                .map(|c| view! { <option value=c.id.to_string()>{c.name}</option> })
                                .collect_view()
                        })
                }}
            </select>

            <PeriodPicker />

            {move || {
                (!app.filters.with(Filters::is_empty))
                    .then(|| {
                        view! {
                            <button class="link" on:click=move |_| app.filters.set(Filters::default())>
                                "Clear"
                            </button>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn PeriodPicker() -> impl IntoView {
    let app = state();
    let presets: [(&str, &str, Period); 6] = [
        ("all", "All time", Period::All),
        ("this-month", "This month", Period::ThisMonth),
        ("last-month", "Last month", Period::LastMonth),
        ("3-months", "Last 3 months", Period::Last3Months),
        ("this-year", "This year", Period::ThisYear),
        ("last-year", "Last year", Period::LastYear),
    ];
    let preset_list = presets.clone();
    let value = move || {
        app.filters.with(|f| match &f.period {
            Period::Month(m) => format!("month:{m}"),
            Period::Custom { .. } => "custom".to_owned(),
            p => preset_list
                .iter()
                .find(|(_, _, q)| q == p)
                .map(|(k, _, _)| (*k).to_owned())
                .unwrap_or_default(),
        })
    };
    let lookup = presets.clone();
    let on_change = move |e: ev::Event| {
        let key = event_target_value(&e);
        let period = if key == "custom" {
            let (from, to) = app.filters.with_untracked(|f| f.period.range());
            Period::Custom {
                from: from.unwrap_or_default(),
                to: to.unwrap_or_else(format::today),
            }
        } else if let Some(m) = key.strip_prefix("month:") {
            Period::Month(m.to_owned())
        } else {
            lookup
                .iter()
                .find(|(k, _, _)| *k == key)
                .map(|(_, _, p)| p.clone())
                .unwrap_or_default()
        };
        app.filters.update(|f| f.period = period);
    };
    let custom = move || match app.filters.with(|f| f.period.clone()) {
        Period::Custom { from, to } => Some((from, to)),
        _ => None,
    };
    let set_custom = move |from: Option<String>, to: Option<String>| {
        app.filters.update(|f| {
            if let Period::Custom { from: a, to: b } = &mut f.period {
                if let Some(v) = from {
                    *a = v;
                }
                if let Some(v) = to {
                    *b = v;
                }
            }
        });
    };
    view! {
        <select aria-label="Period" on:change=on_change prop:value=value>
            {presets.iter().map(|(k, label, _)| view! { <option value=*k>{*label}</option> }).collect_view()}
            {move || {
                app.filters
                    .with(|f| match &f.period {
                        Period::Month(m) => {
                            Some(view! { <option value=format!("month:{m}")>{format::month_long(m)}</option> })
                        }
                        _ => None,
                    })
            }}
            <option value="custom">"Custom range…"</option>
        </select>
        {move || {
            custom()
                .map(|(from, to)| {
                    view! {
                        <span class="range">
                            <input
                                type="date"
                                aria-label="From"
                                prop:value=from
                                on:change=move |e| set_custom(Some(event_target_value(&e)), None)
                            />
                            <span class="muted">"–"</span>
                            <input
                                type="date"
                                aria-label="To"
                                prop:value=to
                                on:change=move |e| set_custom(None, Some(event_target_value(&e)))
                            />
                        </span>
                    }
                })
        }}
    }
}

#[component]
fn Table(rows: Vec<TxRow>, selected: RwSignal<Option<TxRow>>) -> impl IntoView {
    let app = state();
    let filtered_account = app.filters.with_untracked(|f| f.account);
    view! {
        <div class="table card" role="table" aria-label="Transactions">
            <div class="thead" role="row">
                <span role="columnheader">"Date"</span>
                <span role="columnheader">"Description"</span>
                <span role="columnheader" class="col-category">"Category"</span>
                <span role="columnheader" class="col-account">"Account"</span>
                <span role="columnheader" class="num">"Amount"</span>
            </div>
            {rows
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    let counterparty =
                        counterparty(&row).map(str::to_owned).filter(|c| !same_text(c, &row.description));
                    let own = own_account(&row, filtered_account).to_owned();
                    let row_for_click = row.clone();
                    view! {
                        <button
                            id=format!("tx-{id}")
                            class="trow"
                            role="row"
                            class:selected=move || selected.with(|s| s.as_ref().is_some_and(|r| r.id == id))
                            on:click=move |_| {
                                let same = selected.with_untracked(|s| s.as_ref().is_some_and(|r| r.id == id));
                                selected.set(if same { None } else { Some(row_for_click.clone()) });
                            }
                        >
                            <span role="cell" class="cell-date">{format::date(&row.date)}</span>
                            <span role="cell" class="cell-desc">
                                <span class="row-title">{row.description.clone()}</span>
                                {counterparty.map(|c| view! { <span class="row-sub">{c}</span> })}
                            </span>
                            <span role="cell" class="col-category">
                                {match &row.category {
                                    Some(c) => view! { <span class="chip">{c.name.clone()}</span> }.into_any(),
                                    None if row.kind == TxKind::Withdrawal => {
                                        view! { <span class="chip warn">"Uncategorised"</span> }.into_any()
                                    }
                                    None => view! { <span class="chip quiet">{row.kind.label()}</span> }.into_any(),
                                }}
                            </span>
                            <span role="cell" class="col-account muted">{own}</span>
                            <span role="cell" class="num">
                                <Amount
                                    amount=row.amount.amount
                                    currency=row.amount.currency.clone()
                                    signed=true
                                    tone=row.kind != TxKind::Transfer
                                />
                            </span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Equal ignoring case and spacing, or one contains the other.
fn same_text(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (a, b) = (norm(a), norm(b));
    a.contains(&b) || b.contains(&a)
}

/// The other party: where money went for spending, where it came from for
/// income. `None` when the bank did not name it.
fn counterparty(row: &TxRow) -> Option<&str> {
    let name = match row.kind {
        TxKind::Deposit => &row.source.name,
        _ => &row.destination.name,
    };
    Some(name.as_str()).filter(|n| !n.starts_with("(unknown"))
}

/// Tags the old importer stamped on every row carry no meaning for the owner.
fn visible_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter(|t| !t.starts_with("Data Import on"))
        .cloned()
        .collect()
}

/// The owner's account on this row.
fn own_account(row: &TxRow, filtered: Option<i64>) -> &str {
    match (row.kind, filtered) {
        (_, Some(id)) if row.destination.id == id => &row.destination.name,
        (TxKind::Deposit, _) => &row.destination.name,
        _ => &row.source.name,
    }
}

#[component]
fn Detail(row: TxRow, selected: RwSignal<Option<TxRow>>) -> impl IntoView {
    let app = state();
    let counterparty_name = counterparty(&row).map(str::to_owned);
    let tags = visible_tags(&row.tags);
    let category = row.category.clone();
    let month = row.date[..7].to_owned();
    view! {
        <aside class="detail card" aria-label="Transaction details">
            <div class="detail-head">
                <span class="chip quiet">{row.kind.label()}</span>
                <button class="icon-button" title="Close (Esc)" on:click=move |_| selected.set(None)>
                    {icons::close()}
                </button>
            </div>
            <div class="detail-amount">
                <Amount
                    amount=row.amount.amount
                    currency=row.amount.currency.clone()
                    signed=true
                    tone=row.kind != TxKind::Transfer
                />
            </div>
            {row
                .foreign
                .clone()
                .map(|f| {
                    view! {
                        <div class="muted">
                            "Originally " <Amount amount=f.amount currency=f.currency />
                        </div>
                    }
                })}
            <h2 class="detail-title">{row.description.clone()}</h2>
            <dl class="facts">
                <dt>"Date"</dt>
                <dd>{format::date(&row.date)}</dd>
                <dt>"From"</dt>
                <dd>{row.source.name.clone()}</dd>
                <dt>"To"</dt>
                <dd>{row.destination.name.clone()}</dd>
                <dt>"Category"</dt>
                <dd>{row.category.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| "None".into())}</dd>
                {row.bill.clone().map(|b| view! { <dt>"Bill"</dt> <dd>{b}</dd> })}
                {(!tags.is_empty())
                    .then(|| {
                        view! {
                            <dt>"Tags"</dt>
                            <dd class="tags">
                                {tags.into_iter().map(|t| view! { <span class="chip">{t}</span> }).collect_view()}
                            </dd>
                        }
                    })}
                {row.notes.clone().map(|n| view! { <dt>"Notes"</dt> <dd class="notes">{n}</dd> })}
                <dt>"Reconciled"</dt>
                <dd>{if row.reconciled { "Yes" } else { "No" }}</dd>
            </dl>
            <div class="detail-actions">
                {counterparty_name
                    .map(|name| {
                        let label = format!("All with {name}");
                        view! {
                            <button
                                class="button"
                                on:click=move |_| {
                                    app.show_transactions(Filters { text: name.clone(), ..Filters::default() })
                                }
                            >
                                {label}
                            </button>
                        }
                    })}
                {category
                    .map(|c| {
                        view! {
                            <button
                                class="button"
                                on:click=move |_| {
                                    app.show_transactions(Filters {
                                        category: CategoryFilter::Id(c.id),
                                        period: Period::Month(month.clone()),
                                        ..Filters::default()
                                    })
                                }
                            >
                                {format!("{} in {}", c.name, format::month(&row.date[..7]))}
                            </button>
                        }
                    })}
            </div>
        </aside>
    }
}
