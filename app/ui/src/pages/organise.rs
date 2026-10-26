//! Spending → Organise categories (personal-site `integrated-finance.tsx`
//! "Organise categories" branch: `spending-organization.tsx`, the bulk
//! payments card and `owner-category-groups.tsx`), plus the category library
//! (`category-consolidation.tsx`), which the site shows in the transaction
//! inbox.

use finance_types::gateway::GroupMapping;
use finance_types::report::catalog::{
    BulkPaymentSummary, CategoryCatalog, ConsolidationPreview, ConsolidationResult,
};
use finance_types::report::{Currency, Dashboard};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::invoke;
use crate::ds::{
    Badge, Button, CONTROL, Card, CardHeader, CardTitle, CardTone, Eyebrow, Field, Heading, Icon,
    Input, Lucide, Money, Select, Size, Tone, Variant, cn, money,
};
use crate::pages::{plural, with_report};
use crate::{Tab, state};

/// The built-in groups, in the order the owner sees them; custom groups follow.
const GROUPS: [&str; 13] = [
    "Subscriptions & recurring",
    "Groceries",
    "Eating out",
    "Home & household",
    "Housing & utilities",
    "Transport",
    "Travel",
    "Entertainment",
    "Shopping",
    "Health & personal care",
    "Education & work",
    "Fees & taxes",
    "Other",
];

#[component]
pub fn Organise() -> impl IntoView {
    let app = state();
    let loaded = move || app.report.data.with(|d| matches!(d, Some(Ok(_))));
    view! {
        {with_report(|report| view! {
            <SpendingOrganization report=report.clone() />
            <BulkPayments payments=report.bulk_payments.clone() />

        })}
        <Show when=loaded>
            <div id="spending-category-groups"><OwnerCategoryGroups /></div>
        </Show>
    }
}

/// One cell of the coverage strip (`Metric`).
fn metric(label: String, value: AnyView, note: Option<String>) -> impl IntoView {
    view! {
        <div class="grid gap-1 border-line py-3 sm:border-r sm:px-3 sm:first:pl-0 sm:last:border-r-0 sm:last:pr-0">
            <small class="text-xs text-fg-muted">{label}</small>
            <strong class="text-base font-semibold">{value}</strong>
            {note.map(|n| view! { <span class="text-xs text-fg-muted">{n}</span> })}
        </div>
    }
}

/// "Category coverage" (`SpendingOrganization`).
#[component]
fn SpendingOrganization(report: Dashboard) -> impl IntoView {
    let app = state();
    let rows = report
        .monthly_spending
        .currencies
        .into_iter()
        .map(|row| {
            let count = |grouped: bool, categorised: Option<bool>| -> u64 {
                row.groups
                    .iter()
                    .filter(|g| g.group.is_some() == grouped)
                    .flat_map(|g| g.stores.iter())
                    .filter(|s| categorised.is_none_or(|c| s.category.is_some() == c))
                    .map(|s| s.transaction_count)
                    .sum()
            };
            let organized = count(true, None);
            let needs_group = count(false, Some(true));
            let unclassified = count(false, Some(false));
            let active = row.groups.iter().filter(|g| g.group.is_some()).count();
            let unit = row.currency.clone();
            view! {
                <div class="grid grid-cols-2 divide-y divide-line border-y border-line sm:grid-cols-4 sm:divide-y-0">
                    {metric(
                        format!("Net spending · {}", unit.code),
                        view! { <Money value=row.spending unit=unit.clone() neutral=true /> }.into_any(),
                        None,
                    )}
                    {metric("Grouped".into(), organized.to_string().into_any(), Some(format!("{active} active groups")))}
                    {metric("Needs group".into(), needs_group.to_string().into_any(), None)}
                    {metric("Uncategorised".into(), unclassified.to_string().into_any(), None)}
                </div>
            }
        })
        .collect_view();
    view! {
        <Card>
            <CardHeader>
                <div>
                    <Eyebrow level=Heading::P>"THIS MONTH"</Eyebrow>
                    <CardTitle level=Heading::H2>"Category coverage"</CardTitle>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    <Button
                        variant=Variant::Secondary
                        size=Size::Sm
                        on_click=Callback::new(|_| {
                            if let Some(el) = document().get_element_by_id("spending-category-groups") {
                                el.scroll_into_view();
                            }
                        })
                    >
                        "Organise"
                    </Button>
                    <Button variant=Variant::Ghost size=Size::Sm on_click=Callback::new(move |_| app.go(Tab::Transactions))>
                        "Review"
                    </Button>
                </div>
            </CardHeader>
            {rows}
        </Card>
    }
}

/// "Bulk payments · Purchases paid together later".
#[component]
fn BulkPayments(payments: Vec<BulkPaymentSummary>) -> impl IntoView {
    let app = state();
    let body = if payments.is_empty() {
        view! {
            <p class="text-sm text-fg-muted">
                "No bulk-payment links yet. Open Transactions, expand Details & tags, and choose the bulk-payment role."
            </p>
        }
        .into_any()
    } else {
        let cards = payments
            .into_iter()
            .map(|payment| {
                let unit: Currency = payment.currency.clone();
                let components = format!(
                    "{} · {}",
                    plural(payment.component_count as usize, "component"),
                    money(payment.component_total, &unit)
                );
                let settlements = format!(
                    "{} · {}",
                    plural(payment.settlement_count as usize, "settlement"),
                    money(payment.settlement_total, &unit)
                );
                let outstanding = payment.outstanding_amount;
                let state = if outstanding.is_zero() {
                    "Balanced".to_owned()
                } else if outstanding > Decimal::ZERO {
                    format!("{} not settled", money(outstanding, &unit))
                } else {
                    format!("{} needs components", money(outstanding.abs(), &unit))
                };
                view! {
                    <Card tone=CardTone::Muted class="grid gap-1.5 p-4 sm:p-4">
                        <strong class="font-semibold">{payment.name}</strong>
                        <span class="text-sm text-fg-muted">{components}</span>
                        <span class="text-sm text-fg-muted">{settlements}</span>
                        <span class="text-sm font-medium text-accent">{state}</span>
                    </Card>
                }
            })
            .collect_view();
        view! { <div class="grid gap-3 sm:grid-cols-[repeat(auto-fit,minmax(210px,1fr))]">{cards}</div> }
            .into_any()
    };
    view! {
        <Card class="my-6">
            <CardHeader>
                <div>
                    <Eyebrow level=Heading::P>"Bulk payments"</Eyebrow>
                    <CardTitle level=Heading::H2 class="mt-1">"Purchases paid together later"</CardTitle>
                </div>
                <Button variant=Variant::Ghost size=Size::Sm on_click=Callback::new(move |_| app.go(Tab::Transactions))>
                    "Mark transactions →"
                </Button>
            </CardHeader>
            {body}
        </Card>
    }
}

#[derive(Serialize)]
struct GroupArgs {
    category: String,
    group: Option<String>,
    revision: u32,
}

#[derive(Serialize)]
struct NewGroupArgs {
    group: String,
}

/// "Organise spending" (`OwnerCategoryGroups`). Its state lives outside the
/// report so a refresh keeps the search, filter and open rows.
#[component]
fn OwnerCategoryGroups() -> impl IntoView {
    let app = state();
    let mappings = Memo::new(move |_| {
        app.report.data.with(|d| match d {
            Some(Ok(r)) => r.monthly_spending.mappings.clone(),
            _ => Vec::new(),
        })
    });
    let options = Memo::new(move |_| {
        let mut all: Vec<String> = GROUPS.iter().map(|g| (*g).to_owned()).collect();
        app.report.data.with(|d| {
            if let Some(Ok(r)) = d {
                let more = r.custom_spending_groups.iter().chain(
                    r.monthly_spending
                        .mappings
                        .iter()
                        .filter_map(|m| m.group.as_ref()),
                );
                for name in more {
                    if !all.contains(name) {
                        all.push(name.clone());
                    }
                }
            }
        });
        all
    });
    let query = RwSignal::new(String::new());
    let filter = RwSignal::new("needs".to_owned());
    let limit = RwSignal::new(25_usize);
    let new_group = RwSignal::new(String::new());
    let creating = RwSignal::new(false);
    let group_message = RwSignal::new(String::new());
    Effect::new(move |_| {
        query.track();
        filter.track();
        limit.set(25);
    });
    let filtered = Memo::new(move |_| {
        let needle = query.get().trim().to_lowercase();
        let filter = filter.get();
        mappings
            .get()
            .into_iter()
            .filter(|m| match filter.as_str() {
                "all" => true,
                "needs" => m.group.is_none(),
                _ => m.group.is_some(),
            })
            .filter(|m| {
                format!("{} {}", m.category, m.group.as_deref().unwrap_or(""))
                    .to_lowercase()
                    .contains(&needle)
            })
            .collect::<Vec<_>>()
    });
    let create = move || {
        creating.set(true);
        group_message.set(String::new());
        let group = new_group.get_untracked().trim().to_owned();
        spawn_local(async move {
            match invoke::<String>("create_spending_group", &NewGroupArgs { group }).await {
                Ok(name) => {
                    app.refresh();
                    new_group.set(String::new());
                    group_message.set(format!("{name} is available in the category dropdowns."));
                }
                Err(e) => group_message.set(e),
            }
            creating.set(false);
        });
    };
    let badge = move || {
        let all = mappings.get();
        let assigned = all.iter().filter(|m| m.group.is_some()).count();
        format!("{assigned} of {} grouped", all.len())
    };
    let filter_options = vec![
        ("needs".to_owned(), "Needs a group".to_owned()),
        ("assigned".to_owned(), "Grouped".to_owned()),
        ("all".to_owned(), "All categories".to_owned()),
    ];
    view! {
        <Card class="grid gap-4">
            <CardHeader class="mb-0">
                <div>
                    <Eyebrow level=Heading::P>"YOUR CATEGORY GROUPS"</Eyebrow>
                    <CardTitle level=Heading::H2>"Organise spending"</CardTitle>
                </div>
                <Badge tone=Tone::Outline>{badge}</Badge>
            </CardHeader>
            <form
                class="flex flex-wrap items-end gap-3"
                on:submit=move |e| {
                    e.prevent_default();
                    create();
                }
            >
                <Field label="New spending group" id="new-spending-group" class="min-w-0 flex-1 basis-56">
                    <input
                        id="new-spending-group"
                        class=CONTROL
                        required
                        maxlength="64"
                        placeholder="e.g. Pets or Family"
                        prop:value=move || new_group.get()
                        on:input=move |e| new_group.set(event_target_value(&e))
                    />
                </Field>
                <Button submit=true disabled=Signal::derive(move || creating.get() || new_group.get().trim().is_empty())>
                    {move || if creating.get() { "Adding…" } else { "Add group" }}
                </Button>
            </form>
            {move || {
                let m = group_message.get();
                (!m.is_empty()).then(|| view! { <p role="status" class="text-sm text-fg-muted">{m}</p> })
            }}
            <div class="grid gap-3 sm:grid-cols-2">
                <Input value=query kind="search" label="Search categories" placeholder="Find a category or group" />
                <Select value=filter options=Signal::derive(move || filter_options.clone()) label="Filter category groups" />
            </div>
            <p class="text-sm text-fg-muted">
                {move || {
                    let total = filtered.with(Vec::len);
                    format!("Showing {} of {total} matching categories", limit.get().min(total))
                }}
            </p>
            <div>
                <For
                    each=move || filtered.get().into_iter().take(limit.get())
                    key=|m| format!("{}:{}", m.category, m.revision)
                    children=move |m| view! { <GroupRow mapping=m options=options /> }
                />
            </div>
            {move || filtered.with(Vec::is_empty).then(|| view! {
                <p class="text-sm text-fg-muted">"No categories match this view."</p>
            })}
            {move || (filtered.with(Vec::len) > limit.get()).then(|| view! {
                <Button
                    variant=Variant::Secondary
                    class="justify-self-start"
                    on_click=Callback::new(move |_| limit.update(|l| *l += 25))
                >
                    "Show 25 more"
                </Button>
            })}
        </Card>
    }
}

/// One category and its group (`GroupRow`).
#[component]
fn GroupRow(mapping: GroupMapping, options: Memo<Vec<String>>) -> impl IntoView {
    let app = state();
    let category = mapping.category.clone();
    let group = RwSignal::new(mapping.group.clone().unwrap_or_default());
    let revision = RwSignal::new(mapping.revision);
    let busy = RwSignal::new(false);
    let message = RwSignal::new(String::new());
    let suggestion_note = RwSignal::new(String::new());
    let category_for_save = StoredValue::new(category.clone());
    let save = move |next: String| {
        let previous = group.get_untracked();
        group.set(next.clone());
        busy.set(true);
        message.set("Saving…".into());
        let args = GroupArgs {
            category: category_for_save.get_value(),
            group: (!next.is_empty()).then_some(next),
            revision: revision.get_untracked(),
        };
        spawn_local(async move {
            match invoke::<()>("save_spending_group", &args).await {
                Ok(()) => {
                    revision.update(|r| *r += 1);
                    message.set("Saved".into());
                    busy.set(false);
                    app.refresh();
                }
                Err(e) => {
                    group.set(previous);
                    message.set(e);
                    busy.set(false);
                }
            }
        });
    };
    // Group suggestions came from a language model on the server; the app
    // has none, so this shows the site's unavailable state.
    let suggest = move |_| suggestion_note.set("Suggestions are unavailable.".into());
    let label = format!("Spending group for {category}");
    view! {
        <div class="grid items-center gap-x-3 gap-y-2 border-t border-line py-3 [grid-template-columns:1fr_1fr] sm:[grid-template-columns:minmax(150px,1fr)_minmax(160px,1fr)_auto_auto]">
            <strong class="col-span-2 truncate sm:col-span-1">{category}</strong>
            <div class="relative w-full">
                <select
                    aria-label=label
                    class=cn([CONTROL, "appearance-none pr-9", "col-span-2 sm:col-span-1"])
                    disabled=move || busy.get()
                    prop:value=move || group.get()
                    on:change=move |e| save(event_target_value(&e))
                >
                    <option value="" selected=group.get_untracked().is_empty()>"Needs a group"</option>
                    {move || {
                        let current = group.get_untracked();
                        options
                            .get()
                            .into_iter()
                            .map(|item| {
                                let selected = item == current;
                                view! { <option value=item.clone() selected=selected>{item.clone()}</option> }
                            })
                            .collect_view()
                    }}
                </select>
                <span class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-fg-muted">
                    <Lucide icon=Icon::ChevronDown size=16 />
                </span>
            </div>
            <Button size=Size::Sm variant=Variant::Secondary disabled=Signal::derive(move || busy.get()) on_click=Callback::new(suggest)>
                "Suggest"
            </Button>
            {move || {
                let m = message.get();
                (!m.is_empty()).then(|| view! { <span role="status" class="text-xs text-fg-muted">{m}</span> })
            }}
            {move || {
                let note = suggestion_note.get();
                (!note.is_empty()).then(|| view! {
                    <div class="col-span-full grid gap-2">
                        <p role="status" class="text-sm text-fg-muted">{note}</p>
                        <div class="flex flex-wrap items-center gap-2"></div>
                    </div>
                })
            }}
        </div>
    }
}

// The site shows the category library in the transaction inbox; it is
// exported for the Transactions section to embed.
#[allow(unused_imports)]
pub use library::CategoryConsolidation;

mod library {
    #![allow(dead_code)]
    use super::*;

    #[derive(Clone, Serialize)]
    struct ConsolidateArgs {
        source: String,
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    }

    /// The category library: similar names and merging one into another
    /// (`CategoryConsolidation`). The site shows it in the transaction inbox.
    #[component]
    pub fn CategoryConsolidation() -> impl IntoView {
        let app = state();
        // Read from the shared report, so a refresh keeps the choices and
        // the outcome message on screen.
        let catalog = Memo::new(move |_| {
            app.report.data.with(|d| match d {
                Some(Ok(r)) => r.category_catalog.clone(),
                _ => CategoryCatalog::default(),
            })
        });
        let source = RwSignal::new(String::new());
        let target = RwSignal::new(String::new());
        let preview = RwSignal::new(None::<ConsolidationPreview>);
        let busy = RwSignal::new(false);
        let message = RwSignal::new(String::new());
        let submit = move |apply: bool| {
            let current = preview.get_untracked();
            if busy.get_untracked() || (apply && current.is_none()) {
                return;
            }
            busy.set(true);
            message.set(String::new());
            let args = ConsolidateArgs {
                source: source.get_untracked(),
                target: target.get_untracked(),
                token: if apply {
                    current.map(|p| p.token)
                } else {
                    None
                },
            };
            spawn_local(async move {
                if apply {
                    match invoke::<ConsolidationResult>("apply_category_consolidation", &args).await
                    {
                        Ok(body) => {
                            preview.set(None);
                            message.set(if body.complete {
                            format!(
                                "Consolidated {} transactions into “{}”.{}",
                                body.applied,
                                args.target,
                                if body.refreshed {
                                    ""
                                } else {
                                    " Balances and category lists will update after source refresh."
                                }
                            )
                        } else {
                            format!(
                                "{} of {} updated. {}",
                                body.applied,
                                body.total,
                                body.error.unwrap_or_default()
                            )
                        });
                            app.refresh();
                        }
                        Err(e) => {
                            preview.set(None);
                            message.set(e);
                        }
                    }
                } else {
                    match invoke::<ConsolidationPreview>("preview_category_consolidation", &args)
                        .await
                    {
                        Ok(body) => preview.set(Some(body)),
                        Err(e) => {
                            preview.set(None);
                            message.set(e);
                        }
                    }
                }
                busy.set(false);
            });
        };
        let summary = move || {
            catalog.with(|c| {
                format!(
                    "Category library · {} names · {} similar pairs",
                    c.entries.len(),
                    c.similar.len()
                )
            })
        };
        let choices = Signal::derive(move || {
            let mut choices = vec![(String::new(), "Choose category".to_owned())];
            catalog.with(|c| {
                choices.extend(
                    c.entries
                        .iter()
                        .map(|e| (e.name.clone(), format!("{} ({})", e.name, e.count))),
                );
            });
            choices
        });
        let pairs = move || {
            catalog
                .get()
                .similar
                .into_iter()
                .map(|pair| {
                    let text =
                        format!("{} → {} · {} transactions", pair.source, pair.target, pair.count);
                    let (s, t) = (pair.source.clone(), pair.target.clone());
                    view! {
                        <div class="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-3">
                            <span class="min-w-0 break-words text-sm">
                                {text}<br /><small class="text-xs text-fg-muted">{pair.reason}</small>
                            </span>
                            <Button
                                variant=Variant::Secondary
                                size=Size::Sm
                                disabled=Signal::derive(move || busy.get())
                                on_click=Callback::new(move |_| {
                                    source.set(s.clone());
                                    target.set(t.clone());
                                    preview.set(None);
                                })
                            >
                                "Review pair"
                            </Button>
                        </div>
                    }
                })
                .collect_view()
        };
        // Choosing another category drops a preview made for the old choice.
        Effect::new(move |_| {
            source.track();
            target.track();
            preview.set(None);
        });
        let pickers = [
            ("From", "consolidate-from", source),
            ("Keep", "consolidate-keep", target),
        ]
        .into_iter()
        .map(|(label, id, value)| {
            view! {
                <Field label=label id=id class="min-w-0 flex-1 basis-48">
                    <Select
                        id=id
                        value=value
                        options=choices
                        disabled=Signal::derive(move || busy.get())
                    />
                </Field>
            }
        })
        .collect_view();
        let loaded = move || app.report.data.with(|d| matches!(d, Some(Ok(_))));
        view! {
            <details
                class="rounded-lg border border-border bg-card-muted px-4 py-3"
                class:hidden=move || !loaded()
            >
                <summary class="cursor-pointer text-sm font-semibold">{summary}</summary>
                <div class="mt-3 grid gap-4">
                    <p class="text-sm text-fg-muted">
                        "Checked whenever reports refresh. Review meaning before merging similar names. Extra tags stay attached to their transactions."
                    </p>
                    {pairs}
                    <div class="flex flex-wrap items-end gap-3">
                        {pickers}
                        <Button
                            variant=Variant::Secondary
                            disabled=Signal::derive(move || {
                                let (s, t) = (source.get(), target.get());
                                busy.get() || s.is_empty() || t.is_empty() || s == t
                            })
                            on_click=Callback::new(move |_| submit(false))
                        >
                            "Preview consolidation"
                        </Button>
                    </div>
                    {move || preview.get().map(|p| {
                        let text = format!(
                            "Change {} transactions from “{}” to “{}”? Amounts and saved details stay unchanged.",
                            p.count, p.source, p.target
                        );
                        view! {
                            <div class="grid gap-3 rounded-lg border border-border bg-card-strong p-3 text-sm">
                                <p class="break-words">{text}</p>
                                <Button
                                    class="justify-self-start"
                                    disabled=Signal::derive(move || busy.get())
                                    on_click=Callback::new(move |_| submit(true))
                                >
                                    {move || if busy.get() { "Consolidating…" } else { "Confirm consolidation" }}
                                </Button>
                            </div>
                        }
                    })}
                    {move || {
                        let m = message.get();
                        (!m.is_empty()).then(|| view! { <p role="status" class="text-sm text-fg-muted">{m}</p> })
                    }}
                </div>
            </details>
        }
    }
}
