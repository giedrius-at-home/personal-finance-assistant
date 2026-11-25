//! Overview → Today: on a day that went over the plan, what pushed it over
//! and which purchases could have been skipped, with what to do instead.
//! The same summary reaches the phone in the evening.

use finance_types::notify::DaySummary;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::api::{NoArgs, load};
use crate::ds::patterns::{HINT, MUTED, STATUS_NOTE};
use crate::ds::{Card, CardTitle, Eyebrow, Heading, money};

#[component]
pub fn OverviewToday() -> impl IntoView {
    let summary = load::<DaySummary, _>("day_summary", || NoArgs {});
    let body = |s: DaySummary| {
        let over: Vec<_> = s
            .totals
            .iter()
            .filter(|t| t.over_by > Decimal::ZERO)
            .cloned()
            .collect();
        let headline = if over.is_empty() {
            if s.totals.is_empty() {
                "Nothing spent in your plan groups today.".to_owned()
            } else {
                "Today is within the plan.".to_owned()
            }
        } else {
            format!(
                "Today went {} over the plan.",
                over.iter()
                    .map(|t| money(t.over_by, &t.currency))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )
        };
        let totals = over
            .into_iter()
            .map(|t| {
                let groups = t
                    .groups
                    .iter()
                    .take(3)
                    .map(|g| {
                        format!(
                            "{}: {} today, {} a day left",
                            g.group,
                            money(g.spent, &t.currency),
                            money(g.allowance, &t.currency)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                view! {
                    <p class=MUTED>
                        {format!(
                            "Spent {} against the {} a day the plan allows.",
                            money(t.spent, &t.currency),
                            money(t.allowance, &t.currency)
                        )}
                    </p>
                    <p class=HINT>{groups}</p>
                }
            })
            .collect_view();
        let skipped = (!s.avoidable.is_empty()).then(|| {
            view! {
                <div class="mt-4 grid gap-2">
                    <p class=HINT>"COULD HAVE BEEN AVOIDED"</p>
                    <ul class="grid gap-2">
                        {s
                            .avoidable
                            .iter()
                            .map(|p| {
                                let when = p.time.clone().map(|t| format!(" · {t}")).unwrap_or_default();
                                view! {
                                    <li class="flex items-start justify-between gap-3 border-b border-border pb-2">
                                        <span>
                                            {p.merchant.clone()}
                                            <span class=HINT>{format!(" {}{when}", p.group)}</span>
                                            <span class="block text-xs text-fg-muted">
                                                {format!("Instead: {}", p.instead)}
                                            </span>
                                        </span>
                                        <span class="tabular-nums font-medium">{money(p.amount, &p.currency)}</span>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                </div>
            }
        });
        view! {
            <p class="mt-2 text-lg font-semibold">{headline}</p>
            {totals}
            {skipped}
        }
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"TODAY"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"What today cost"</CardTitle>
            {move || match summary.data.get() {
                None => view! { <p class=STATUS_NOTE>"Loading today…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p class=STATUS_NOTE>{format!("Today's summary is not available ({e}).")}</p> }
                        .into_any()
                }
                Some(Ok(s)) => body(s).into_any(),
            }}
        </Card>
    }
}
