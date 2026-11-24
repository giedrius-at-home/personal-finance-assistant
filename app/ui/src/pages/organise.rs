//! Spending → Organise categories (personal-site `integrated-finance.tsx`
//! "Organise categories" branch: `spending-organization.tsx`, the bulk
//! payments card and `owner-category-groups.tsx`), plus the category library
//! (`category-consolidation.tsx`), which the site shows in the transaction
//! inbox.

use finance_types::gateway::GroupMapping;
use finance_types::report::catalog::BulkPaymentSummary;
use finance_types::report::{Currency, Dashboard};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::invoke;
use crate::ds::patterns::{
    ACTIONS, FORM_FIELD_WIDE, HINT, INLINE_FORM, METRIC_VALUE, MUTED, NUM_HINT, SELECT_CHEVRON,
    SELECT_WRAP, STACK,
};
use crate::ds::{
    Badge, Button, CONTROL, Card, CardHeader, CardTitle, CardTone, Eyebrow, Field, Heading, Icon,
    Input, Lucide, Money, Select, Size, Tone, Variant, cn, money,
};
use crate::layout::{Initial, LayoutState, PageView, PanelDef};
use crate::pages::{plural, report_is, with_report};
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

/// Organise categories' panels, in their first-run order.
pub const PANELS: &[PanelDef] = &[
    PanelDef {
        id: "spending.organise.coverage",
        title: "Category coverage",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || with_report(|r| view! { <SpendingOrganization report=r.clone() /> }).into_any(),
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "spending.organise.bulk",
        title: "Purchases paid together later",
        page: PAGE,
        movable: true,
        initial: Initial::Folded,
        half: false,
        render: || {
            with_report(|r| view! { <BulkPayments payments=r.bulk_payments.clone() /> }).into_any()
        },
        summary: None,
        shown: None,
    },
    PanelDef {
        id: "spending.organise.groups",
        title: "Organise spending",
        page: PAGE,
        movable: true,
        initial: Initial::Open,
        half: false,
        render: || {
            view! { <div id="spending-category-groups"><OwnerCategoryGroups /></div> }.into_any()
        },
        summary: None,
        // The groups wait for the report and draw nothing until it loads.
        shown: Some(|| report_is(|_| true)),
    },
];

const PAGE: &str = "spending.organise";

#[component]
pub fn Organise() -> impl IntoView {
    view! { <PageView tab="spending" tab_name="Spending" page=PAGE /> }
}

/// One cell of the coverage strip (`Metric`).
fn metric(label: String, value: AnyView, note: Option<String>) -> impl IntoView {
    view! {
        <div class="grid gap-1 border-line py-3 sm:border-r sm:px-3 sm:first:pl-0 sm:last:border-r-0 sm:last:pr-0">
            <small class=HINT>{label}</small>
            <strong class=METRIC_VALUE>{value}</strong>
            {note.map(|n| view! { <span class=HINT>{n}</span> })}
        </div>
    }
}

/// "Category coverage" (`SpendingOrganization`).
#[component]
fn SpendingOrganization(report: Dashboard) -> impl IntoView {
    let app = state();
    let layout = use_context::<LayoutState>();
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
                <div class=ACTIONS>
                    <Button
                        variant=Variant::Secondary
                        size=Size::Sm
                        on_click=Callback::new(move |_| {
                            if let Some(layout) = layout {
                                layout.open("spending", "Spending", PAGE, "spending.organise.groups");
                            }
                            // On the next tick, once the opened panel is drawn.
                            set_timeout(
                                || {
                                    if let Some(el) = document().get_element_by_id("spending-category-groups") {
                                        el.scroll_into_view();
                                    }
                                },
                                std::time::Duration::ZERO,
                            );
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
            <p class=MUTED>
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
                        <span class=MUTED>{components}</span>
                        <span class=MUTED>{settlements}</span>
                        <span class="text-sm font-medium text-accent">{state}</span>
                    </Card>
                }
            })
            .collect_view();
        view! { <div class="grid gap-3 sm:grid-cols-[repeat(auto-fit,minmax(210px,1fr))]">{cards}</div> }
            .into_any()
    };
    view! {
        <Card>
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
        <Card class=STACK>
            <CardHeader class="mb-0">
                <div>
                    <Eyebrow level=Heading::P>"YOUR CATEGORY GROUPS"</Eyebrow>
                    <CardTitle level=Heading::H2>"Organise spending"</CardTitle>
                </div>
                <Badge tone=Tone::Outline>{badge}</Badge>
            </CardHeader>
            <form
                class=INLINE_FORM
                on:submit=move |e| {
                    e.prevent_default();
                    create();
                }
            >
                <Field label="New spending group" id="new-spending-group" class=FORM_FIELD_WIDE>
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
                (!m.is_empty()).then(|| view! { <p role="status" class=MUTED>{m}</p> })
            }}
            <div class="grid gap-3 sm:grid-cols-2">
                <Input value=query kind="search" label="Search categories" placeholder="Find a category or group" />
                <Select value=filter options=Signal::derive(move || filter_options.clone()) label="Filter category groups" />
            </div>
            <p class=NUM_HINT>
                {move || {
                    let total = filtered.with(Vec::len);
                    format!("{} / {total}", limit.get().min(total))
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
                <p class=MUTED>"No categories match this view."</p>
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

#[derive(Serialize)]
struct SuggestArgs {
    category: String,
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
    // `/api/finance/group-suggest`: ranked options from the configured model.
    let suggesting = RwSignal::new(false);
    let suggestions = RwSignal::new(Vec::<String>::new());
    let category_for_suggest = StoredValue::new(category.clone());
    let suggest = move |_| {
        suggesting.set(true);
        suggestion_note.set(String::new());
        let args = SuggestArgs {
            category: category_for_suggest.get_value(),
        };
        spawn_local(async move {
            match invoke::<finance_types::ai::GroupSuggestion>("suggest_spending_group", &args)
                .await
            {
                Ok(body) => {
                    let mut groups: Vec<String> =
                        body.options.into_iter().map(|o| o.group).collect();
                    // A local model answers with one group and no ranking.
                    if groups.is_empty()
                        && let Some(group) = body.group
                    {
                        groups.push(group);
                    }
                    suggestion_note.set(
                        if groups.is_empty() {
                            "No suggestion found."
                        } else {
                            "Choose a group."
                        }
                        .into(),
                    );
                    suggestions.set(groups);
                }
                Err(e) => suggestion_note.set(e),
            }
            suggesting.set(false);
        });
    };
    let label = format!("Spending group for {category}");
    view! {
        <div class="grid items-center gap-x-3 gap-y-2 border-t border-line py-3 [grid-template-columns:1fr_1fr] sm:[grid-template-columns:minmax(150px,1fr)_minmax(160px,1fr)_auto_auto]">
            <strong class="col-span-2 truncate sm:col-span-1">{category}</strong>
            <div class=SELECT_WRAP>
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
                <span class=SELECT_CHEVRON>
                    <Lucide icon=Icon::ChevronDown size=16 />
                </span>
            </div>
            <Button size=Size::Sm variant=Variant::Secondary disabled=Signal::derive(move || busy.get() || suggesting.get()) on_click=Callback::new(suggest)>
                {move || if suggesting.get() { "Finding…" } else { "Suggest" }}
            </Button>
            {move || {
                let m = message.get();
                (!m.is_empty()).then(|| view! { <span role="status" class=HINT>{m}</span> })
            }}
            {move || {
                let note = suggestion_note.get();
                (!note.is_empty() || !suggestions.with(Vec::is_empty)).then(|| view! {
                    <div class="col-span-full grid gap-2">
                        <p role="status" class=MUTED>{note}</p>
                        <div class=ACTIONS>
                            {suggestions
                                .get()
                                .into_iter()
                                .map(|option| {
                                    let known = options.with(|o| o.contains(&option));
                                    let pick = option.clone();
                                    view! {
                                        <Button
                                            size=Size::Sm
                                            variant=Variant::Secondary
                                            disabled=Signal::derive(move || busy.get() || !known)
                                            on_click=Callback::new(move |_| save(pick.clone()))
                                        >
                                            {option}
                                        </Button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    </div>
                })
            }}
        </div>
    }
}
