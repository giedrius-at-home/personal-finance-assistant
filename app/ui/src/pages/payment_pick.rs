//! "Which payment paid this?": the bank payments, newest first, those
//! close to the asked amount on top. Shared by the boards a box is dragged
//! to "paid" on.

use finance_types::report::review::{
    SearchDirection, SearchRow, TransactionSearchQuery, TransactionSearchResult,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::invoke;
use crate::ds::patterns::{BADGE_TIGHT, DATE, ERROR_LINE, PAYMENT_ROW, TRUNCATE};
use crate::ds::{Badge, Button, CONTROL, Popup, Size, Tone, Variant, money};
use crate::format;

/// What needs a payment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The plan or wish asked about.
    pub id: String,
    pub name: String,
    /// The currency code the payment must be in.
    pub currency: String,
    /// Money received, not paid.
    pub incoming: bool,
    /// What it was planned to cost; zero when there is nothing to compare.
    pub amount: Decimal,
    /// Payments to leave out; `None`: those that already pay a plan.
    pub taken: Option<Vec<String>>,
}

#[derive(Serialize)]
struct SearchArgs {
    query: TransactionSearchQuery,
}

/// How many payments the list offers at once.
const FOUND: u32 = 200;

/// A plan is not exact to the cent: this near counts as the same amount.
pub fn close(planned: Decimal, paid: Decimal) -> bool {
    !planned.is_zero() && (planned - paid).abs() <= Decimal::TWO
}

/// The payments to offer: those in `currency`, the close ones first, each
/// run newest first as the search gave them. `taken` already pay a plan.
pub fn offered(
    rows: Vec<SearchRow>,
    currency: &str,
    planned: Decimal,
    taken: &[String],
) -> Vec<SearchRow> {
    let mut rows: Vec<SearchRow> = rows
        .into_iter()
        .filter(|r| r.amount.currency.code == currency)
        .filter(|r| !taken.contains(&format!("firefly:{}", r.transaction_id)))
        .collect();
    rows.sort_by_key(|r| !close(planned, r.amount.amount));
    rows
}

/// Open while `ask` holds something; `on_pick` gets it with the payment.
#[component]
pub fn PaymentPicker(
    ask: RwSignal<Option<Ask>>,
    #[prop(into)] busy: Signal<bool>,
    on_pick: Callback<(Ask, SearchRow)>,
    /// What to do when no payment fits.
    #[prop(into)]
    otherwise: String,
) -> impl IntoView {
    let app = crate::state();
    let query = RwSignal::new(String::new());
    let found = RwSignal::new(Vec::<SearchRow>::new());
    let error = RwSignal::new(String::new());
    let asked = StoredValue::new(0_u32);
    Effect::new(move |_| {
        let Some(a) = ask.get() else {
            return;
        };
        let q = query.get();
        asked.update_value(|n| *n += 1);
        let mine = asked.get_value();
        let query = TransactionSearchQuery {
            q,
            account: None,
            from: None,
            to: None,
            direction: Some(if a.incoming {
                SearchDirection::In
            } else {
                SearchDirection::Out
            }),
            min: None,
            max: None,
            page: 1,
            page_size: FOUND,
        };
        spawn_local(async move {
            let result =
                invoke::<TransactionSearchResult>("search_transactions", &SearchArgs { query })
                    .await;
            // A later keystroke has asked again: its answer is the one to show.
            if asked.get_value() != mine {
                return;
            }
            match result {
                Ok(result) => {
                    error.set(String::new());
                    // What pays a plan already is not offered again.
                    let taken: Vec<String> = a.taken.clone().unwrap_or_else(|| {
                        app.report.data.with_untracked(|d| {
                            d.as_ref()
                                .and_then(|r| r.as_ref().ok())
                                .map(|r| {
                                    r.planned_payments
                                        .iter()
                                        .filter_map(|p| p.transaction_id.clone())
                                        .collect()
                                })
                                .unwrap_or_default()
                        })
                    });
                    found.set(offered(result.items, &a.currency, a.amount, &taken));
                }
                Err(e) => error.set(e),
            }
        });
    });
    let shut = Callback::new(move |()| {
        ask.set(None);
        query.set(String::new());
        found.set(Vec::new());
    });
    let otherwise = StoredValue::new(otherwise);
    move || {
        let a = ask.get()?;
        let planned = a.amount;
        let title = format!("Which payment paid {}?", a.name);
        let a = StoredValue::new(a);
        Some(view! {
            <Popup
                title=title
                description=otherwise.get_value()
                class="w-[min(94vw,44rem)]"
                on_close=shut
            >
                <input
                    type="text"
                    aria-label="Narrow the payments"
                    placeholder="Narrow the payments: a shop, a name"
                    maxlength="128"
                    class=CONTROL
                    prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                />
                {move || {
                    let text = error.get();
                    (!text.is_empty()).then(|| view! { <p role="alert" class=ERROR_LINE>{text}</p> })
                }}
                <ul class="max-h-[55vh] overflow-y-auto">
                    {move || {
                        found
                            .get()
                            .into_iter()
                            .map(|row| {
                                let text = [row.counterparty.clone(), row.description.clone()]
                                    .into_iter()
                                    .flatten()
                                    .find(|s| !s.is_empty())
                                    .unwrap_or_else(|| "Payment".to_owned());
                                let near = close(planned, row.amount.amount);
                                let value = money(row.amount.amount, &row.amount.currency);
                                let day = format::date(&row.occurred_on);
                                let title = text.clone();
                                let pick = move |_| on_pick.run((a.get_value(), row.clone()));
                                view! {
                                    <li class=PAYMENT_ROW>
                                        <span class=DATE>{day}</span>
                                        <span class="flex min-w-0 items-center gap-2">
                                            <span class=TRUNCATE title=title>{text}</span>
                                            {near.then(|| view! { <Badge tone=Tone::Positive class=BADGE_TIGHT>"Same amount"</Badge> })}
                                        </span>
                                        <span class="tabular-nums">{value}</span>
                                        <Button size=Size::Sm variant=Variant::Secondary disabled=busy on_click=Callback::new(pick)>
                                            "This one"
                                        </Button>
                                    </li>
                                }
                            })
                            .collect_view()
                    }}
                    <Show when=move || found.with(Vec::is_empty)>
                        <li class="py-2 text-sm text-fg-muted">"No payment found."</li>
                    </Show>
                </ul>
            </Popup>
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::{Currency, UnitMoney};

    fn row(id: &str, amount: &str, code: &str) -> SearchRow {
        SearchRow {
            transaction_id: id.into(),
            occurred_on: "2026-11-21".into(),
            direction: SearchDirection::Out,
            amount: UnitMoney {
                amount: amount.parse().unwrap(),
                currency: Currency::new(code),
            },
            counterparty: None,
            description: None,
            account_id: "1".into(),
            account_name: None,
            category: None,
            firefly_group_id: None,
        }
    }

    #[test]
    fn a_euro_or_two_off_is_still_the_same_amount() {
        let planned: Decimal = "566".parse().unwrap();
        let rows = vec![
            row("far", "12", "EUR"),
            row("more", "567.80", "EUR"),
            row("usd", "566", "USD"),
            row("less", "564", "EUR"),
            row("over", "568.01", "EUR"),
            row("9", "566", "EUR"),
        ];
        let ids: Vec<String> = offered(rows, "EUR", planned, &["firefly:9".to_owned()])
            .into_iter()
            .map(|r| r.transaction_id)
            .collect();
        // The close ones first, the rest in the order given, no other currency.
        assert_eq!(ids, ["more", "less", "far", "over"]);
    }
}
