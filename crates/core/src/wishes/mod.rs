//! Planned purchases and whether the budget can afford them.

pub mod baseline;
pub mod ladder;
pub mod schedule;
pub mod store;

use finance_types::gateway::{BudgetPlan, Wish, WishSettings, WishStatus};
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::wishes::{Verdict, WishMonth, WishRow, WishesView};
use finance_types::report::{Currency, Dashboard};
use ladder::Demand;
use rust_decimal::Decimal;
use schedule::Family;

/// The wishes screen, judged against the dashboard's allocation.
pub fn wishes_view(dashboard: &Dashboard, wishes: &[Wish], settings: &WishSettings) -> WishesView {
    let alloc = dashboard.monthly_allocation.as_ref().and_then(|m| {
        m.currencies
            .iter()
            .find(|c| c.currency.code == "EUR")
            .or_else(|| m.currencies.first())
    });
    let plans: Vec<BudgetPlan> = dashboard
        .monthly_budgets
        .iter()
        .map(|b| b.plan.clone())
        .collect();
    // The card bill is already counted in next month's fixed costs.
    let card_bill = settings
        .card_bill_id
        .as_deref()
        .and_then(|bill_id| {
            dashboard
                .next_month_subscriptions
                .as_ref()
                .unwrap_or(&dashboard.subscriptions)
                .rows
                .iter()
                .find(|row| row.bill_id == bill_id)
        })
        .map_or(Decimal::ZERO, |row| row.amount_max);
    view_from(alloc, &plans, card_bill, wishes, settings)
}

/// The wishes screen, judged against one currency's allocation.
pub fn view_from(
    alloc: Option<&CurrencyAllocation>,
    plans: &[BudgetPlan],
    card_bill: Decimal,
    wishes: &[Wish],
    settings: &WishSettings,
) -> WishesView {
    let judged = alloc.and_then(|a| {
        let month0 = baseline::current_month(a)?;
        let (items, families) = schedule::plan_items(wishes, month0);
        let months = schedule::horizon(&items, month0);
        let demands = schedule::demands(&items, month0, months);
        let (bases, card_stale) = baseline::baselines(
            a,
            plans,
            card_bill,
            settings,
            schedule::earmarked(wishes),
            months,
        );
        let judged = ladder::judge(&bases, &demands, settings.card_limit);
        Some((judged, families, demands, card_stale))
    });
    let currency = alloc.map_or_else(
        || {
            wishes
                .first()
                .map_or_else(Currency::eur, |w| w.currency.clone())
        },
        |a| a.currency.clone(),
    );
    let (months, families, demands, card_stale) = judged.unwrap_or_default();
    let ctx = RowContext {
        wishes,
        months: &months,
        families: &families,
        month0: demands.first().map_or(&[], Vec::as_slice),
    };
    let mut tops: Vec<&Wish> = wishes.iter().filter(|w| w.parent_id.is_none()).collect();
    sort_by_due(&mut tops);
    WishesView {
        currency,
        settings: settings.clone(),
        card_stale,
        wishes: tops.into_iter().map(|w| ctx.row(w)).collect(),
        months,
    }
}

fn sort_by_due(wishes: &mut [&Wish]) {
    wishes.sort_by(|a, b| (&a.due_on, &a.created_at).cmp(&(&b.due_on, &b.created_at)));
}

struct RowContext<'a> {
    wishes: &'a [Wish],
    months: &'a [WishMonth],
    families: &'a [Family],
    month0: &'a [Demand],
}

impl RowContext<'_> {
    fn verdicts<'v>(&'v self, wish_id: &'v str) -> impl Iterator<Item = &'v Verdict> {
        self.months
            .iter()
            .flat_map(|m| &m.verdicts)
            .filter(move |v| v.wish_id.as_deref() == Some(wish_id))
    }

    fn set_aside(&self, wish_id: &str) -> Decimal {
        self.month0
            .iter()
            .filter(|d| d.wish_id.as_deref() == Some(wish_id) && d.label.starts_with("Saving for "))
            .map(|d| d.amount)
            .sum()
    }

    fn row(&self, wish: &Wish) -> WishRow {
        let mut subs: Vec<&Wish> = self
            .wishes
            .iter()
            .filter(|w| w.parent_id.as_deref() == Some(wish.wish_id.as_str()))
            .collect();
        sort_by_due(&mut subs);
        let children: Vec<WishRow> = subs.into_iter().map(|w| self.row(w)).collect();
        let family = self.families.iter().find(|f| f.parent_id == wish.wish_id);
        let (mut short, mut tradeoffs, mut monthly_set_aside) =
            (Decimal::ZERO, false, Decimal::ZERO);
        if wish.status == WishStatus::Planned {
            for v in self.verdicts(&wish.wish_id) {
                short += v.short;
                tradeoffs |= !v.cuts.is_empty() || v.on_card > Decimal::ZERO;
            }
            monthly_set_aside = self.set_aside(&wish.wish_id);
            for child in &children {
                short += child.short;
                tradeoffs |= child.tradeoffs;
                monthly_set_aside += child.monthly_set_aside;
            }
        }
        WishRow {
            wish: wish.clone(),
            children,
            rest: family.map_or(Decimal::ZERO, |f| f.rest),
            over_cap: family.is_some_and(|f| f.over_cap),
            monthly_set_aside,
            short,
            tradeoffs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::baseline::tests::{alloc, d, plan, plan_group, row};
    use super::*;
    use finance_types::gateway::{WishSource, WishStatus};
    use finance_types::report::Currency;

    fn wish(id: &str, parent: Option<&str>, amount: &str, due: &str) -> Wish {
        Wish {
            wish_id: id.into(),
            parent_id: parent.map(String::from),
            name: id.into(),
            amount: d(amount),
            currency: Currency::eur(),
            due_on: due.into(),
            source: WishSource::Account {
                account_id: "a".into(),
            },
            saved: Decimal::ZERO,
            status: WishStatus::Planned,
            bought: None,
            created_at: format!("2026-01-01T00:00:{id}"),
        }
    }

    fn going_out_plan() -> Vec<BudgetPlan> {
        vec![plan(
            "2026-10-01",
            vec![plan_group("g-out", "Going out", Some(2), None)],
        )]
    }

    #[test]
    fn view_end_to_end() {
        let a = alloc(
            "100",
            "0",
            vec![row("Going out", "120", "0")],
            vec![row("Going out", "120", "0")],
        );
        let view = view_from(
            Some(&a),
            &going_out_plan(),
            Decimal::ZERO,
            &[wish("tv", None, "150", "2026-10-20")],
            &WishSettings::default(),
        );
        assert_eq!(view.currency, Currency::eur());
        assert_eq!(view.months.len(), 2);
        assert_eq!(view.wishes.len(), 1);
        let r = &view.wishes[0];
        assert!(r.tradeoffs);
        assert_eq!(r.short, Decimal::ZERO);
        let v = &view.months[0].verdicts[0];
        assert_eq!(v.from_free, d("100"));
        assert_eq!(v.cuts.len(), 1);
        assert_eq!(v.cuts[0].group_id, "g-out");
        assert_eq!(v.cuts[0].from, d("120"));
        assert_eq!(v.cuts[0].to, d("70"));
    }

    #[test]
    fn parent_row_aggregates_children_and_rest() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut gone = wish("gone", None, "10", "2026-09-01");
        gone.status = WishStatus::Dropped;
        let ws = [
            wish("trip", None, "1000", "2026-12-31"),
            wish("hotel", Some("trip"), "400", "2026-11-10"),
            wish("later", None, "60", "2026-12-01"),
            gone,
        ];
        let s = WishSettings {
            card_limit: Decimal::ZERO,
            ..WishSettings::default()
        };
        let view = view_from(Some(&a), &[], Decimal::ZERO, &ws, &s);
        let ids: Vec<_> = view
            .wishes
            .iter()
            .map(|r| r.wish.wish_id.as_str())
            .collect();
        assert_eq!(ids, vec!["gone", "later", "trip"]);
        let gone = &view.wishes[0];
        assert_eq!(gone.short, Decimal::ZERO);
        assert!(!gone.tradeoffs);
        let trip = &view.wishes[2];
        assert_eq!(trip.children.len(), 1);
        assert_eq!(trip.rest, d("600"));
        assert!(!trip.over_cap);
        // Nothing is affordable: trip's short is its rest plus its hotel.
        assert_eq!(trip.children[0].short, d("400"));
        assert_eq!(trip.short, d("1000"));
        // Month 0 set-asides: hotel 400/2, rest 600/3.
        assert_eq!(trip.children[0].monthly_set_aside, d("200"));
        assert_eq!(trip.monthly_set_aside, d("400"));
        assert_eq!(view.wishes[1].monthly_set_aside, d("20"));
    }

    #[test]
    fn no_allocation_leaves_wishes_unjudged() {
        let view = view_from(
            None,
            &[],
            Decimal::ZERO,
            &[wish("tv", None, "150", "2026-10-20")],
            &WishSettings::default(),
        );
        assert!(view.months.is_empty());
        assert_eq!(view.wishes.len(), 1);
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
        assert!(!view.wishes[0].tradeoffs);
        assert_eq!(view.currency, Currency::eur());
    }
}
