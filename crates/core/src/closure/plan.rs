//! The money plan beside the closure: from the balances as they are now,
//! what each account must keep, what the pockets should receive, what the
//! salary account can send on, and what the savings account must hold for
//! the wishes already saved.

use std::collections::HashMap;

use finance_types::gateway::{AccountRole, BudgetPlan, Direction, Wish, WishKind, WishStatus};
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::closure::{
    AccountRoleKind, MoneyPlan, PlanAccount, PlanMove, PredictedBill, SavingsStanding,
};
use finance_types::report::plans::{PlanStatus, PlannedPaymentView};
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::model::Account;

/// What [`money_plan`] reads.
#[derive(Debug)]
pub struct PlanInputs<'a> {
    pub as_of: Date,
    /// The EUR allocation Overview shows (its `current_month` is the month
    /// being funded now).
    pub allocation: &'a CurrencyAllocation,
    /// The month's stored budget plan, for each group's home account.
    pub budget: Option<&'a BudgetPlan>,
    pub payments: &'a [PlannedPaymentView],
    /// The balance-check accounts (active, spending or savings), with their
    /// ledger balances.
    pub accounts: &'a [&'a Account],
    pub roles: &'a [AccountRole],
    pub wishes: &'a [Wish],
    pub salary_account_id: Option<&'a str>,
    /// Unplanned spending expected for the rest of the month (the outlook's
    /// estimate from past months); kept back from the wiggle room.
    pub expected_unplanned: Decimal,
    /// Varying bills predicted from past months; one still to pay this
    /// month is kept on its account.
    pub predicted_bills: &'a [PredictedBill],
}

/// Σ `saved` of planned one-off wishes in `currency`: money that must stay
/// in savings.
pub fn wishes_must_keep(wishes: &[Wish], currency: &str) -> Vec<(String, Decimal)> {
    let mut out: Vec<(String, Decimal)> = wishes
        .iter()
        .filter(|w| {
            w.status == WishStatus::Planned
                && w.kind == WishKind::OneOff
                && w.currency.code == currency
                && w.saved > Decimal::ZERO
        })
        .map(|w| (w.name.clone(), w.saved))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn day_of(text: &str) -> Option<Date> {
    text.get(..10)?.parse().ok()
}

/// What the planned one-off wishes in `currency` still need saved (a
/// parent's cap covers its parts).
pub fn wishes_open(wishes: &[Wish], currency: &str) -> Decimal {
    wishes
        .iter()
        .filter(|w| {
            w.status == WishStatus::Planned
                && w.kind == WishKind::OneOff
                && w.parent_id.is_none()
                && w.currency.code == currency
        })
        .map(|w| (w.amount - w.saved).max(Decimal::ZERO))
        .sum()
}

/// Where the money should go now; `None` without an allocation currency
/// to plan in.
pub fn money_plan(input: &PlanInputs<'_>) -> MoneyPlan {
    let allocation = input.allocation;
    let currency = &allocation.currency;
    let code = currency.code.as_str();
    let month = day_of(&allocation.current_month).unwrap_or(input.as_of.first_of_month());
    let role_of =
        |id: &str| -> Option<&AccountRole> { input.roles.iter().find(|r| r.account_id == id) };
    let kind_of = |id: &str| {
        role_of(id).map_or(AccountRoleKind::Unclassified, |r| {
            AccountRoleKind::parse(&r.role)
        })
    };
    let remaining_of: HashMap<&str, Decimal> = allocation
        .groups
        .iter()
        .map(|g| (g.group.as_str(), g.remaining))
        .collect();

    let mut accounts: Vec<PlanAccount> = Vec::new();
    for account in input.accounts.iter().filter(|a| a.currency().code == code) {
        let id = account.account_id.as_str();
        let role = kind_of(id);
        let mut parts: Vec<(String, Decimal)> = Vec::new();
        if role == AccountRoleKind::Spending {
            if input.salary_account_id == Some(id)
                && allocation.current_fixed_remaining > Decimal::ZERO
            {
                parts.push((
                    "Bills still to pay".to_owned(),
                    allocation.current_fixed_remaining,
                ));
            }
            let plans: Decimal = input
                .payments
                .iter()
                .filter(|p| {
                    p.direction == Direction::Outflow
                        && p.status != PlanStatus::Paid
                        && p.account_id.as_deref() == Some(id)
                        && p.amount.currency.code == code
                        && day_of(&p.due_on).is_some_and(|d| {
                            d.first_of_month() == month || (p.pending && d < month)
                        })
                })
                .map(|p| p.amount.amount)
                .sum();
            if plans > Decimal::ZERO {
                parts.push(("Planned payments".to_owned(), plans));
            }
            for bill in input
                .predicted_bills
                .iter()
                .filter(|b| !b.paid_this_month && b.account_id == id && b.amount > Decimal::ZERO)
            {
                parts.push((format!("{} (predicted)", bill.category), bill.amount));
            }
            for group in input.budget.map(|b| b.groups.as_slice()).unwrap_or(&[]) {
                if group.account_id.as_deref() != Some(id) {
                    continue;
                }
                let left = remaining_of
                    .get(group.name.as_str())
                    .copied()
                    .unwrap_or(group.amount)
                    .max(Decimal::ZERO);
                if left > Decimal::ZERO {
                    parts.push((format!("{} budget left", group.name), left));
                }
            }
        }
        let keep: Decimal = parts.iter().map(|(_, v)| *v).sum();
        let balance = account.balance.amount;
        let salary = input.salary_account_id == Some(id);
        accounts.push(PlanAccount {
            account_id: id.to_owned(),
            name: account.name.clone(),
            role,
            balance,
            keep,
            keep_parts: parts,
            needs: if role == AccountRoleKind::Spending && !salary {
                (keep - balance).max(Decimal::ZERO)
            } else {
                Decimal::ZERO
            },
            spare: (balance - keep).max(Decimal::ZERO),
            short: (keep - balance).max(Decimal::ZERO),
        });
    }

    // The savings account the rest goes to: the one reached via a hub,
    // else the first by name.
    let mut savings: Vec<&PlanAccount> = accounts
        .iter()
        .filter(|a| a.role == AccountRoleKind::Savings)
        .collect();
    savings.sort_by(|a, b| {
        let via = |a: &PlanAccount| {
            role_of(&a.account_id)
                .and_then(|r| r.via_account_id.as_ref())
                .is_none()
        };
        via(a).cmp(&via(b)).then_with(|| a.name.cmp(&b.name))
    });
    let savings_target = savings.first().map(|a| a.account_id.clone());

    let mut moves = Vec::new();
    let mut incoming = Decimal::ZERO;
    if let Some(source) = input
        .salary_account_id
        .and_then(|id| accounts.iter().find(|a| a.account_id == id))
    {
        let mut left = source.spare;
        let mut needy: Vec<&PlanAccount> = accounts
            .iter()
            .filter(|a| a.needs > Decimal::ZERO && a.account_id != source.account_id)
            .collect();
        needy.sort_by(|a, b| a.name.cmp(&b.name));
        for target in needy {
            if left <= Decimal::ZERO {
                break;
            }
            let amount = target.needs.min(left);
            left -= amount;
            moves.push(PlanMove {
                from_account_id: source.account_id.clone(),
                to_account_id: target.account_id.clone(),
                via_account_id: role_of(&target.account_id).and_then(|r| r.via_account_id.clone()),
                amount,
                reason: format!("Top up {}", target.name),
                cover: false,
            });
        }
        if left > Decimal::ZERO
            && let Some(target) = &savings_target
            && *target != source.account_id
        {
            incoming = left;
            moves.push(PlanMove {
                from_account_id: source.account_id.clone(),
                to_account_id: target.clone(),
                via_account_id: role_of(target).and_then(|r| r.via_account_id.clone()),
                amount: left,
                reason: "Savings".to_owned(),
                cover: false,
            });
        }
    }

    let wishes = wishes_must_keep(input.wishes, code);
    let must_keep: Decimal = wishes.iter().map(|(_, v)| *v).sum();
    // What each spending account is still short after the moves so far:
    // the salary account's bills first, then the pockets by name.
    let open_shorts = |moves: &[PlanMove]| -> Vec<(usize, Decimal)> {
        let mut open: Vec<(usize, Decimal)> = accounts
            .iter()
            .enumerate()
            .filter(|(_, a)| a.role == AccountRoleKind::Spending)
            .filter_map(|(i, a)| {
                let sent: Decimal = moves
                    .iter()
                    .filter(|m| m.to_account_id == a.account_id)
                    .map(|m| m.amount)
                    .sum();
                let left = a.short - sent;
                (left > Decimal::ZERO).then_some((i, left))
            })
            .collect();
        open.sort_by(|(a, _), (b, _)| {
            let pocket =
                |i: &usize| input.salary_account_id != Some(accounts[*i].account_id.as_str());
            pocket(a)
                .cmp(&pocket(b))
                .then_with(|| accounts[*a].name.cmp(&accounts[*b].name))
        });
        open
    };
    let cover = |from: &PlanAccount, to: &PlanAccount, amount: Decimal, reason: String| PlanMove {
        from_account_id: from.account_id.clone(),
        to_account_id: to.account_id.clone(),
        via_account_id: role_of(&to.account_id).and_then(|r| r.via_account_id.clone()),
        amount,
        reason,
        cover: true,
    };
    // Spare money on the other spending accounts covers a short one first
    // (the salary account's spare is already handed out above).
    let mut spare: Vec<(usize, Decimal)> = accounts
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            a.role == AccountRoleKind::Spending
                && a.spare > Decimal::ZERO
                && input.salary_account_id != Some(a.account_id.as_str())
        })
        .map(|(i, a)| (i, a.spare))
        .collect();
    spare.sort_by(|(a, x), (b, y)| {
        y.cmp(x)
            .then_with(|| accounts[*a].name.cmp(&accounts[*b].name))
    });
    for (target, mut left) in open_shorts(&moves) {
        for (source, room) in spare.iter_mut() {
            if left <= Decimal::ZERO {
                break;
            }
            let amount = left.min(*room);
            if amount <= Decimal::ZERO {
                continue;
            }
            *room -= amount;
            left -= amount;
            let (from, to) = (&accounts[*source], &accounts[target]);
            moves.push(cover(
                from,
                to,
                amount,
                format!("Cover {} from {}", to.name, from.name),
            ));
        }
    }
    // What is still open comes back out of savings, as far as the room
    // above the wishes and the estimate reaches.
    let mut outgoing = Decimal::ZERO;
    if let Some(source) = savings_target
        .as_ref()
        .and_then(|id| accounts.iter().find(|a| a.account_id == *id))
    {
        let mut room = (source.balance - must_keep - input.expected_unplanned.max(Decimal::ZERO))
            .max(Decimal::ZERO);
        for (target, left) in open_shorts(&moves) {
            if room <= Decimal::ZERO {
                break;
            }
            let amount = left.min(room);
            room -= amount;
            outgoing += amount;
            let to = &accounts[target];
            moves.push(cover(
                source,
                to,
                amount,
                format!("Cover {} from savings", to.name),
            ));
        }
    }
    let mut overspent: Vec<(String, Decimal)> = allocation
        .groups
        .iter()
        .filter(|g| g.spent > g.limit)
        .map(|g| (g.group.clone(), g.spent - g.limit))
        .collect();
    overspent.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let savings = savings_target.and_then(|id| {
        let account = accounts.iter().find(|a| a.account_id == id)?;
        let expected: Vec<(String, Decimal)> = (input.expected_unplanned > Decimal::ZERO)
            .then(|| {
                (
                    "Unplanned spending (estimate)".to_owned(),
                    input.expected_unplanned,
                )
            })
            .into_iter()
            .collect();
        let set_aside: Decimal = expected.iter().map(|(_, v)| *v).sum();
        Some(SavingsStanding {
            account_id: account.account_id.clone(),
            name: account.name.clone(),
            balance: account.balance,
            incoming,
            outgoing,
            must_keep,
            wishes,
            wishes_open: wishes_open(input.wishes, code),
            expected,
            wiggle_room: account.balance + incoming - outgoing - must_keep - set_aside,
        })
    });

    MoneyPlan {
        as_of: input.as_of.to_string(),
        month: month.to_string(),
        currency: currency.clone(),
        accounts,
        moves,
        savings,
        predicted_bills: input.predicted_bills.to_vec(),
        overspent,
    }
}

#[cfg(test)]
mod tests {
    use finance_types::gateway::{BudgetGroup, WishSource};
    use finance_types::report::allocation::GroupAllocation;
    use finance_types::report::{Currency, UnitMoney};
    use jiff::civil::date;

    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn account(id: &str, name: &str, balance: &str) -> Account {
        Account {
            account_id: id.to_owned(),
            ledger_id: 0,
            name: name.to_owned(),
            balance: UnitMoney {
                amount: d(balance),
                currency: Currency::eur(),
            },
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    fn role(id: &str, role: &str, via: Option<&str>) -> AccountRole {
        AccountRole {
            account_id: id.to_owned(),
            role: role.to_owned(),
            goal_id: None,
            via_account_id: via.map(str::to_owned),
            confirmed_at: String::new(),
        }
    }

    fn wish(name: &str, saved: &str, status: WishStatus) -> Wish {
        Wish {
            wish_id: name.to_owned(),
            parent_id: None,
            name: name.to_owned(),
            amount: d("500"),
            currency: Currency::eur(),
            due_on: "2026-12-01".into(),
            source: WishSource::Card,
            saved: d(saved),
            saved_month: None,
            status,
            bought: None,
            kind: WishKind::OneOff,
            must: false,
            created_at: String::new(),
            event: None,
        }
    }

    fn allocation() -> CurrencyAllocation {
        let mut a = crate::wishes::baseline::tests::alloc("0", "0", Vec::new(), Vec::new());
        a.current_month = "2026-11-01".into();
        a.current_fixed_remaining = d("1020");
        a.groups = vec![
            GroupAllocation {
                group: "Groceries".into(),
                limit: d("300"),
                spent: d("0"),
                remaining: d("300"),
                projected_month_end: d("0"),
            },
            GroupAllocation {
                group: "Fun".into(),
                limit: d("100"),
                spent: d("40"),
                remaining: d("60"),
                projected_month_end: d("0"),
            },
        ];
        a
    }

    fn budget() -> BudgetPlan {
        let group = |name: &str, amount: &str, account: &str| BudgetGroup {
            id: name.to_lowercase(),
            name: name.to_owned(),
            amount: d(amount),
            source_groups: Vec::new(),
            categories: Vec::new(),
            transaction_ids: Vec::new(),
            account_id: Some(account.to_owned()),
            importance: None,
            floor: None,
        };
        BudgetPlan {
            month: "2026-11-01".into(),
            currency: Currency::eur(),
            revision: 1,
            groups: vec![
                group("Groceries", "300", "firefly:415"),
                group("Fun", "100", "firefly:416"),
            ],
        }
    }

    fn plan(accounts: &[&Account], roles: &[AccountRole], wishes: &[Wish]) -> MoneyPlan {
        let allocation = allocation();
        let budget = budget();
        money_plan(&PlanInputs {
            as_of: date(2026, 11, 1),
            allocation: &allocation,
            budget: Some(&budget),
            payments: &[],
            accounts,
            roles,
            wishes,
            salary_account_id: Some("firefly:1"),
            expected_unplanned: Decimal::ZERO,
            predicted_bills: &[],
        })
    }

    fn setup() -> (Vec<Account>, Vec<AccountRole>) {
        (
            vec![
                account("firefly:1", "ABN AMRO", "2000"),
                account("firefly:5", "Swedbank", "7"),
                account("firefly:76", "Revolut", "0"),
                account("firefly:415", "Revolut Groceries", "120"),
                account("firefly:416", "Revolut Fun out", "60"),
                account("firefly:413", "Revolut Savings", "1200"),
            ],
            vec![
                role("firefly:1", "spending", None),
                role("firefly:5", "spending", None),
                role("firefly:76", "spending", None),
                role("firefly:415", "spending", Some("firefly:76")),
                role("firefly:416", "spending", Some("firefly:76")),
                role("firefly:413", "savings", Some("firefly:76")),
            ],
        )
    }

    #[test]
    fn salary_account_keeps_the_bills_and_sends_the_rest() {
        let (accounts, roles) = setup();
        let refs: Vec<&Account> = accounts.iter().collect();
        let plan = plan(&refs, &roles, &[]);
        let abn = &plan.accounts[0];
        assert_eq!(abn.keep, d("1020"));
        assert_eq!(abn.spare, d("980"));
        assert_eq!(abn.needs, Decimal::ZERO);
        // Groceries needs 180 to reach its 300 left; Fun already holds its 60.
        let groceries = plan
            .accounts
            .iter()
            .find(|a| a.name == "Revolut Groceries")
            .unwrap();
        assert_eq!(groceries.needs, d("180"));
        let fun = plan
            .accounts
            .iter()
            .find(|a| a.name == "Revolut Fun out")
            .unwrap();
        assert_eq!(fun.needs, Decimal::ZERO);
        assert_eq!(
            plan.moves
                .iter()
                .map(|m| (
                    m.to_account_id.as_str(),
                    m.amount,
                    m.via_account_id.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("firefly:415", d("180"), Some("firefly:76")),
                ("firefly:413", d("800"), Some("firefly:76")),
            ]
        );
        let savings = plan.savings.unwrap();
        assert_eq!(savings.incoming, d("800"));
        assert_eq!(savings.wiggle_room, d("2000"));
    }

    #[test]
    fn estimates_are_kept_back_and_an_unpaid_prediction_stays_on_its_account() {
        let (accounts, roles) = setup();
        let refs: Vec<&Account> = accounts.iter().collect();
        let allocation = allocation();
        let budget = budget();
        let bill = |paid: bool| PredictedBill {
            category: "NS".into(),
            account_id: "firefly:1".into(),
            amount: d("28.50"),
            due_on: "2026-11-10".into(),
            paid_this_month: paid,
            method: "TimesFM".into(),
            history: vec![d("20"), d("31")],
        };
        let make = |bills: &[PredictedBill]| {
            money_plan(&PlanInputs {
                as_of: date(2026, 11, 1),
                allocation: &allocation,
                budget: Some(&budget),
                payments: &[],
                accounts: &refs,
                roles: &roles,
                wishes: &[],
                salary_account_id: Some("firefly:1"),
                expected_unplanned: d("150"),
                predicted_bills: bills,
            })
        };
        let unpaid = make(&[bill(false)]);
        let abn = &unpaid.accounts[0];
        assert_eq!(abn.keep, d("1048.50"));
        assert!(
            abn.keep_parts
                .contains(&("NS (predicted)".to_owned(), d("28.50")))
        );
        let savings = unpaid.savings.unwrap();
        assert_eq!(
            savings.expected,
            vec![("Unplanned spending (estimate)".to_owned(), d("150"))]
        );
        // 1200 + 771.50 incoming − 150 estimate.
        assert_eq!(savings.wiggle_room, d("1821.50"));
        assert_eq!(unpaid.predicted_bills.len(), 1);
        let paid = make(&[bill(true)]);
        assert_eq!(paid.accounts[0].keep, d("1020"));
    }

    #[test]
    fn other_spending_accounts_are_never_a_source() {
        let (accounts, roles) = setup();
        let refs: Vec<&Account> = accounts.iter().collect();
        let plan = plan(&refs, &roles, &[]);
        let swedbank = plan.accounts.iter().find(|a| a.name == "Swedbank").unwrap();
        assert_eq!(swedbank.spare, d("7"));
        assert!(plan.moves.iter().all(|m| m.from_account_id == "firefly:1"));
    }

    #[test]
    fn a_short_salary_account_sends_nothing_on() {
        let (mut accounts, roles) = setup();
        accounts[0].balance.amount = d("1000.08");
        let refs: Vec<&Account> = accounts.iter().collect();
        let plan = plan(&refs, &roles, &[]);
        assert_eq!(plan.accounts[0].short, d("19.92"));
        // Spare money and savings give: nothing leaves the salary account.
        assert!(
            plan.moves
                .iter()
                .all(|m| m.cover && m.from_account_id != "firefly:1")
        );
        // Swedbank's spare 7 first, then savings for the bills' other 12.92.
        let to_bills: Vec<Decimal> = plan
            .moves
            .iter()
            .filter(|m| m.to_account_id == "firefly:1")
            .map(|m| m.amount)
            .collect();
        assert_eq!(to_bills, [d("7"), d("12.92")]);
        assert_eq!(plan.savings.unwrap().incoming, Decimal::ZERO);
    }

    #[test]
    fn must_keep_counts_planned_one_off_wishes_only() {
        let (accounts, roles) = setup();
        let refs: Vec<&Account> = accounts.iter().collect();
        let wishes = vec![
            wish("Dinner", "300", WishStatus::Planned),
            wish("Zoo", "98", WishStatus::Planned),
            wish("Old", "50", WishStatus::Bought),
            wish("Gone", "20", WishStatus::Dropped),
            wish("Empty", "0", WishStatus::Planned),
        ];
        let plan = plan(&refs, &roles, &wishes);
        let savings = plan.savings.unwrap();
        assert_eq!(savings.must_keep, d("398"));
        assert_eq!(
            savings.wishes,
            vec![("Dinner".to_owned(), d("300")), ("Zoo".to_owned(), d("98"))]
        );
        assert_eq!(savings.wiggle_room, d("1200") + d("800") - d("398"));
    }

    #[test]
    fn savings_cover_what_the_salary_account_is_short() {
        let (mut accounts, roles) = setup();
        // Bills need 1018.90 on ABN and Groceries wants its 300; ABN holds
        // too little for either.
        accounts[0].balance.amount = d("900");
        let refs: Vec<&Account> = accounts.iter().collect();
        let plan = plan(&refs, &roles, &[wish("Dinner", "300", WishStatus::Planned)]);
        let from_savings: Vec<(&str, Decimal)> = plan
            .moves
            .iter()
            .filter(|m| m.from_account_id == "firefly:413")
            .map(|m| (m.to_account_id.as_str(), m.amount))
            .collect();
        let short = |id: &str| {
            let a = plan.accounts.iter().find(|a| a.account_id == id).unwrap();
            a.short
        };
        // Swedbank's spare 7 goes to the bills first; savings cover the rest.
        let spare: Vec<(&str, &str, Decimal)> = plan
            .moves
            .iter()
            .filter(|m| m.cover && m.from_account_id != "firefly:413")
            .map(|m| {
                (
                    m.from_account_id.as_str(),
                    m.to_account_id.as_str(),
                    m.amount,
                )
            })
            .collect();
        assert_eq!(spare, [("firefly:5", "firefly:1", d("7"))]);
        assert_eq!(from_savings[0], ("firefly:1", short("firefly:1") - d("7")));
        let total: Decimal = from_savings.iter().map(|(_, v)| *v).sum();
        let savings = plan.savings.unwrap();
        assert_eq!(savings.outgoing, total);
        assert_eq!(savings.wiggle_room, d("1200") - d("300") - total);
        // Never below what the wishes saved.
        assert!(savings.wiggle_room >= Decimal::ZERO);
    }

    #[test]
    fn wiggle_room_goes_negative_when_wishes_need_more() {
        let (mut accounts, roles) = setup();
        accounts[0].balance.amount = d("1020");
        accounts[5].balance.amount = d("100");
        let refs: Vec<&Account> = accounts.iter().collect();
        let plan = plan(&refs, &roles, &[wish("Dinner", "300", WishStatus::Planned)]);
        assert_eq!(plan.savings.unwrap().wiggle_room, d("-200"));
    }
}
