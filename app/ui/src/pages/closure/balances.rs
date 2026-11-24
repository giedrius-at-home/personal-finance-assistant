//! The sync step's balance check (not on the site): each checked account's
//! ledger balance next to the bank's, confirmed one by one before the split.

use finance_types::report::Currency;
use finance_types::report::closure::{BalanceState, BalanceStatus, ClosureStatus};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::Wizard;
use super::ui::{ACTIONS, money};
use crate::ds::patterns::SECTION_BODY;
use crate::ds::{
    Button, Input, Size, TABLE, TABLE_CONTAINER, TCELL, TCELL_NUM, TH, TH_NUM, TROW, Variant, cn,
};

/// `value` written with exactly the currency's minor digits (EUR 12.5 → "12.50").
pub fn with_minor_units(value: Decimal, currency: &Currency) -> String {
    let unit = u32::from(currency.minor_unit);
    let mut value = value.round_dp(unit);
    value.rescale(unit);
    value.to_string()
}

/// What the "Confirm as" field starts with: the bank figure when known, else
/// the ledger's.
pub fn prefill(status: &BalanceStatus) -> String {
    with_minor_units(status.bank.unwrap_or(status.ledger), &status.currency)
}

/// The number input's step: one minor unit (EUR → "0.01", JPY → "1").
pub fn input_step(currency: &Currency) -> String {
    Decimal::new(1, u32::from(currency.minor_unit)).to_string()
}

/// The typed balance as a number.
pub fn parse_balance(typed: &str) -> Result<Decimal, String> {
    typed
        .trim()
        .parse::<Decimal>()
        .map_err(|_| "Enter a number".to_owned())
}

/// Whether the typed figure equals `target`.
pub fn same_figure(typed: &str, target: Decimal) -> bool {
    parse_balance(typed).is_ok_and(|v| v == target)
}

/// Whether a row offers "Confirm": every unconfirmed or stale row, and a
/// confirmed one whose field no longer reads its confirmed balance.
pub fn needs_confirm(status: &BalanceStatus, typed: &str) -> bool {
    match (&status.state, &status.check) {
        (BalanceState::Confirmed, Some(check)) => !same_figure(typed, check.balance),
        (BalanceState::Confirmed, None) => false,
        _ => true,
    }
}

/// The rows "Confirm all matching" posts, in order: not yet confirmed, with
/// the field reading the ledger figure. `typed` pairs with `statuses`.
pub fn matching_rows(statuses: &[BalanceStatus], typed: &[String]) -> Vec<(String, String)> {
    statuses
        .iter()
        .zip(typed)
        .filter(|(s, t)| s.state != BalanceState::Confirmed && same_figure(t, s.ledger))
        .map(|(s, t)| (s.account_id.clone(), t.trim().to_owned()))
        .collect()
}

/// Whether the sync step shows the Balances panel: while the month can still
/// take a confirmed balance (the server accepts one up to Settled).
pub fn shows_balances(status: ClosureStatus) -> bool {
    matches!(
        status,
        ClosureStatus::Open | ClosureStatus::Settling | ClosureStatus::Settled
    )
}

/// The split waits until every account balance reads Confirmed.
pub fn balances_locked(statuses: &[BalanceStatus]) -> bool {
    statuses.iter().any(|b| b.state != BalanceState::Confirmed)
}

/// The bank column: the bank's figure and the day it reported it
/// ("€ 12.50 (27 Oct 2026)"), or a dash when no bank reported one.
pub fn bank_figure(status: &BalanceStatus) -> String {
    match (status.bank, status.bank_reported_on.as_deref()) {
        (None, _) => "—".to_owned(),
        (Some(bank), None) => money(bank, &status.currency),
        (Some(bank), Some(on)) => format!(
            "{} ({})",
            money(bank, &status.currency),
            crate::format::date(on)
        ),
    }
}

/// The status cell: its text and whether it warns.
pub fn status_text(status: &BalanceStatus) -> (String, bool) {
    match status.state {
        BalanceState::Confirmed => ("Confirmed".to_owned(), false),
        BalanceState::Stale => ("Changed since confirmed".to_owned(), true),
        BalanceState::Unconfirmed => match status.bank {
            Some(bank) if bank != status.ledger => (
                format!("Off by {}", money(bank - status.ledger, &status.currency)),
                true,
            ),
            Some(_) => ("Matches".to_owned(), false),
            None => (String::new(), false),
        },
    }
}

/// One row per checked account: ledger, bank, the figure to confirm and
/// where it stands.
#[component]
pub fn BalancesPanel(balances: Vec<BalanceStatus>, disabled: Signal<bool>) -> impl IntoView {
    if balances.is_empty() {
        return None;
    }
    let wizard = expect_context::<Wizard>();
    let inputs: Vec<RwSignal<String>> =
        balances.iter().map(|b| RwSignal::new(prefill(b))).collect();
    let statuses = StoredValue::new(balances.clone());
    let fields = StoredValue::new(inputs.clone());
    let matching = move || {
        let typed: Vec<String> = fields.with_value(|f| f.iter().map(|s| s.get()).collect());
        statuses.with_value(|s| matching_rows(s, &typed))
    };
    let none_matching = Signal::derive(move || disabled.get() || matching().is_empty());
    let rows = balances
        .into_iter()
        .zip(inputs)
        .map(|(status, typed)| {
            let currency = status.currency.clone();
            let (text, warn) = status_text(&status);
            let account_id = StoredValue::new(status.account_id.clone());
            let name = status.name.clone();
            let confirm_label = format!("Confirm {name}");
            let row = StoredValue::new(status.clone());
            let show_confirm = move || row.with_value(|s| typed.with(|t| needs_confirm(s, t)));
            view! {
                <tr class=TROW>
                    <td class=TCELL>{status.name.clone()}</td>
                    <td class=cn([TCELL_NUM, "whitespace-nowrap"])>{money(status.ledger, &currency)}</td>
                    <td class=cn([TCELL_NUM, "whitespace-nowrap"])>
                        {bank_figure(&status)}
                    </td>
                    <td class=cn([TCELL, "min-w-32"])>
                        <Input
                            value=typed
                            kind="number"
                            step=input_step(&currency)
                            label=format!("Confirm {name} as")
                            disabled=disabled
                        />
                    </td>
                    <td class=cn([TCELL, if warn { "text-warning-fg" } else { "text-fg-muted" }])>{text}</td>
                    <td class=TCELL>
                        {move || show_confirm().then(|| view! {
                            <Button
                                variant=Variant::Secondary
                                size=Size::Sm
                                disabled=disabled
                                label=confirm_label.clone()
                                on_click=Callback::new(move |_| {
                                    wizard.confirm_balance(account_id.get_value(), typed.get_untracked());
                                })
                            >
                                "Confirm"
                            </Button>
                        })}
                    </td>
                </tr>
            }
        })
        .collect_view();
    Some(view! {
        <div class=SECTION_BODY>
            <div class=ACTIONS>
                <Button
                    variant=Variant::Secondary
                    size=Size::Sm
                    disabled=none_matching
                    on_click=Callback::new(move |_| wizard.confirm_matching(untrack(matching)))
                >
                    "Confirm all matching"
                </Button>
            </div>
            <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label="Account balances">
                <table class=TABLE data-density="sm">
                    <thead class="text-left">
                        <tr>
                            <th class=TH>"Account"</th>
                            <th class=TH_NUM>"Ledger"</th>
                            <th class=TH_NUM>"Bank"</th>
                            <th class=TH>"Confirm as"</th>
                            <th class=TH>"Status"</th>
                            <th class=TH></th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        </div>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::closure::BalanceCheck;

    fn d(n: i64, scale: u32) -> Decimal {
        Decimal::new(n, scale)
    }

    fn status(
        id: &str,
        ledger: Decimal,
        bank: Option<Decimal>,
        state: BalanceState,
    ) -> BalanceStatus {
        BalanceStatus {
            account_id: id.into(),
            name: id.into(),
            currency: Currency::eur(),
            ledger,
            bank,
            bank_reported_on: None,
            check: (state != BalanceState::Unconfirmed).then(|| BalanceCheck {
                balance: ledger,
                ledger_balance: ledger,
                confirmed_at: "2026-11-01T10:00:00Z".into(),
                transaction_id: None,
            }),
            state,
            adjusted_by: None,
        }
    }

    #[test]
    fn prefill_takes_the_bank_figure_with_minor_units() {
        let s = status("a", d(1000, 2), Some(d(125, 1)), BalanceState::Unconfirmed);
        assert_eq!(prefill(&s), "12.50");
        let s = status("a", d(1000, 0), None, BalanceState::Unconfirmed);
        assert_eq!(prefill(&s), "1000.00");
        let mut s = status("a", d(12345, 3), None, BalanceState::Unconfirmed);
        s.currency = Currency::new("JPY");
        assert_eq!(prefill(&s), "12");
    }

    #[test]
    fn step_is_one_minor_unit() {
        assert_eq!(input_step(&Currency::eur()), "0.01");
        assert_eq!(input_step(&Currency::new("JPY")), "1");
        assert_eq!(input_step(&Currency::new("KWD")), "0.001");
    }

    #[test]
    fn parse_rejects_text() {
        assert_eq!(parse_balance(" 12.30 "), Ok(d(1230, 2)));
        assert_eq!(parse_balance("abc"), Err("Enter a number".to_owned()));
        assert_eq!(parse_balance(""), Err("Enter a number".to_owned()));
    }

    #[test]
    fn matching_skips_confirmed_and_differing_rows() {
        let statuses = vec![
            status("a", d(1000, 2), None, BalanceState::Unconfirmed),
            status("b", d(500, 2), Some(d(600, 2)), BalanceState::Unconfirmed),
            status("c", d(700, 2), None, BalanceState::Confirmed),
            status("d", d(800, 2), None, BalanceState::Stale),
        ];
        let typed = vec!["10".into(), "6.00".into(), "7.00".into(), "8.00".into()];
        assert_eq!(
            matching_rows(&statuses, &typed),
            vec![
                ("a".to_owned(), "10".to_owned()),
                ("d".to_owned(), "8.00".to_owned())
            ]
        );
    }

    #[test]
    fn confirm_button_shows_until_the_field_reads_the_check() {
        let confirmed = status("a", d(1000, 2), None, BalanceState::Confirmed);
        assert!(!needs_confirm(&confirmed, "10.00"));
        assert!(needs_confirm(&confirmed, "11.00"));
        assert!(needs_confirm(&confirmed, "x"));
        let open = status("a", d(1000, 2), None, BalanceState::Unconfirmed);
        assert!(needs_confirm(&open, "10.00"));
    }

    #[test]
    fn split_locks_until_every_row_is_confirmed() {
        assert!(!balances_locked(&[]));
        let ok = status("a", d(1, 0), None, BalanceState::Confirmed);
        assert!(!balances_locked(std::slice::from_ref(&ok)));
        let stale = status("b", d(1, 0), None, BalanceState::Stale);
        assert!(balances_locked(&[ok, stale]));
    }

    #[test]
    fn panel_shows_until_the_month_closes() {
        assert!(shows_balances(ClosureStatus::Open));
        assert!(shows_balances(ClosureStatus::Settling));
        assert!(shows_balances(ClosureStatus::Settled));
        assert!(!shows_balances(ClosureStatus::Closed));
        assert!(!shows_balances(ClosureStatus::ChangedAfterClose));
    }

    #[test]
    fn bank_figure_carries_its_date() {
        let mut s = status("a", d(1000, 2), None, BalanceState::Unconfirmed);
        assert_eq!(bank_figure(&s), "—");
        s.bank = Some(d(1250, 2));
        let figure = money(d(1250, 2), &Currency::eur());
        assert_eq!(bank_figure(&s), figure);
        s.bank_reported_on = Some("2026-10-27".into());
        assert_eq!(bank_figure(&s), format!("{figure} (27 Oct 2026)"));
    }

    #[test]
    fn status_texts() {
        let s = status("a", d(1000, 2), None, BalanceState::Confirmed);
        assert_eq!(status_text(&s), ("Confirmed".into(), false));
        let s = status("a", d(1000, 2), None, BalanceState::Stale);
        assert_eq!(status_text(&s), ("Changed since confirmed".into(), true));
        let s = status("a", d(1000, 2), Some(d(1000, 2)), BalanceState::Unconfirmed);
        assert_eq!(status_text(&s), ("Matches".into(), false));
        let s = status("a", d(1000, 2), None, BalanceState::Unconfirmed);
        assert_eq!(status_text(&s), (String::new(), false));
        let s = status("a", d(1000, 2), Some(d(1250, 2)), BalanceState::Unconfirmed);
        let (text, warn) = status_text(&s);
        assert!(warn);
        assert_eq!(
            text,
            format!("Off by {}", money(d(250, 2), &Currency::eur()))
        );
    }
}
