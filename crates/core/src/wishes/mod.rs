//! Planned purchases and whether the budget can afford them.

pub mod baseline;
pub mod ladder;
pub mod schedule;
pub mod store;

use finance_types::gateway::{
    BudgetPlan, Repeat, Wish, WishKind, WishPurchase, WishSettings, WishSource, WishStatus,
};
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::wishes::{ChangeableBill, Cut, Verdict, WishMonth, WishRow, WishesView};
use finance_types::report::{Currency, Dashboard};
use jiff::civil::Date;
use ladder::Demand;
use rust_decimal::Decimal;
use schedule::{BillNow, Family, Planned};
use std::collections::HashMap;

use crate::load::Bill;

/// The wishes screen, judged against the dashboard's allocation. `bills`
/// are the snapshot's, for the schedules the dashboard rows leave out.
pub fn wishes_view(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    wishes: &[Wish],
    settings: &WishSettings,
) -> WishesView {
    let (plans, card_bill, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        plans,
        card_bill,
        bills: &bills,
    }
    .view(wishes, settings)
}

/// The cuts buying `wish_id` with `purchase` needs in month 0: the unsaved
/// gap judged as a purchase of its own, after everything else planned.
pub fn purchase_cuts(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    wishes: &[Wish],
    settings: &WishSettings,
    wish_id: &str,
    purchase: &WishPurchase,
) -> Vec<Cut> {
    let (plans, card_bill, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        plans,
        card_bill,
        bills: &bills,
    }
    .purchase_cuts(wishes, settings, wish_id, purchase)
}

/// The plans the judging reads: the dashboard's months (stored or default)
/// plus every other stored plan, so a default month inherits the ranking
/// of the latest stored plan that ranks a group.
pub fn view_plans(dashboard: Vec<BudgetPlan>, stored: &[BudgetPlan]) -> Vec<BudgetPlan> {
    let mut plans = dashboard;
    for plan in stored {
        if !plans
            .iter()
            .any(|p| p.month == plan.month && p.currency == plan.currency)
        {
            plans.push(plan.clone());
        }
    }
    plans
}

fn allocation(dashboard: &Dashboard) -> Option<&CurrencyAllocation> {
    dashboard.monthly_allocation.as_ref().and_then(|m| {
        m.currencies
            .iter()
            .find(|c| c.currency.code == "EUR")
            .or_else(|| m.currencies.first())
    })
}

/// The budget plans, the card bill's amount and the bills as they stand.
fn dashboard_parts(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    settings: &WishSettings,
) -> (Vec<BudgetPlan>, Decimal, HashMap<String, BillNow>) {
    let plans = view_plans(
        dashboard
            .monthly_budgets
            .iter()
            .map(|b| b.plan.clone())
            .collect(),
        stored_plans,
    );
    // The card bill is already counted in next month's fixed costs.
    let card_bill = settings
        .first()
        .bill_id
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
    (plans, card_bill, bills_now(dashboard, bills))
}

/// The schedule the judging knows a bill by (Firefly's frequency and skip);
/// `None` when it cannot judge it.
pub fn repeat_of(frequency: &str, skip: u32) -> Option<Repeat> {
    match (frequency.to_lowercase().as_str(), skip) {
        ("weekly", 0) => Some(Repeat::Weekly),
        ("weekly", 1) => Some(Repeat::Biweekly),
        ("monthly", 0) => Some(Repeat::Monthly),
        ("yearly", 0) => Some(Repeat::Yearly),
        _ => None,
    }
}

/// Every bill the judging knows the schedule of, at its highest amount.
fn bills_now(dashboard: &Dashboard, bills: &[Bill]) -> HashMap<String, BillNow> {
    dashboard
        .next_month_subscriptions
        .as_ref()
        .unwrap_or(&dashboard.subscriptions)
        .rows
        .iter()
        .filter_map(|row| {
            let skip = bills
                .iter()
                .find(|b| b.id == row.bill_id)
                .map_or(0, |b| b.skip);
            let every = repeat_of(&row.frequency, skip)?;
            let bill = BillNow {
                amount: row.amount_max,
                every,
            };
            Some((row.bill_id.clone(), bill))
        })
        .collect()
}

/// What the judging reads besides the wishes.
#[derive(Debug)]
pub struct Inputs<'a> {
    pub alloc: Option<&'a CurrencyAllocation>,
    pub plans: Vec<BudgetPlan>,
    pub card_bill: Decimal,
    pub bills: &'a HashMap<String, BillNow>,
}

impl Inputs<'_> {
    /// Judge `items` from month 0 on.
    fn judge(
        &self,
        alloc: &CurrencyAllocation,
        month0: Date,
        items: &[Planned],
        wishes: &[Wish],
        settings: &WishSettings,
    ) -> (Vec<WishMonth>, Vec<Vec<Demand>>, bool) {
        let months = schedule::horizon(items, month0);
        let demands = schedule::demands(items, month0, months);
        let (bases, card_stale) = baseline::baselines(
            alloc,
            &self.plans,
            self.card_bill,
            settings,
            schedule::earmarked(wishes),
            months,
        );
        let judged = ladder::judge(&bases, &demands, settings.first().limit);
        (judged, demands, card_stale)
    }

    /// The wishes screen.
    pub fn view(&self, wishes: &[Wish], settings: &WishSettings) -> WishesView {
        let judged = self.alloc.and_then(|a| {
            let month0 = baseline::current_month(a)?;
            let (items, families) = schedule::plan_items(wishes, month0, self.bills);
            let (judged, demands, card_stale) = self.judge(a, month0, &items, wishes, settings);
            Some((judged, families, demands, card_stale))
        });
        let currency = self.alloc.map_or_else(
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
            bills: self.bills,
        };
        let mut tops: Vec<&Wish> = wishes.iter().filter(|w| w.parent_id.is_none()).collect();
        sort_by_due(&mut tops);
        let mut changeable_bills: Vec<ChangeableBill> = self
            .bills
            .iter()
            .map(|(id, bill)| ChangeableBill {
                bill_id: id.clone(),
                every: bill.every,
            })
            .collect();
        changeable_bills.sort_by(|a, b| a.bill_id.cmp(&b.bill_id));
        WishesView {
            currency,
            settings: settings.clone(),
            card_stale,
            stale_cards: Vec::new(),
            wishes: tops.into_iter().map(|w| ctx.row(w)).collect(),
            months,
            changeable_bills,
        }
    }

    /// See [`purchase_cuts`].
    pub fn purchase_cuts(
        &self,
        wishes: &[Wish],
        settings: &WishSettings,
        wish_id: &str,
        purchase: &WishPurchase,
    ) -> Vec<Cut> {
        let Some(alloc) = self.alloc else {
            return Vec::new();
        };
        let (Some(month0), Some(wish)) = (
            baseline::current_month(alloc),
            wishes.iter().find(|w| w.wish_id == wish_id),
        ) else {
            return Vec::new();
        };
        let gap = purchase.amount - wish.saved;
        if gap <= Decimal::ZERO {
            return Vec::new();
        }
        let (mut items, _) = schedule::plan_items(wishes, month0, self.bills);
        items.retain(|i| i.wish_id != wish_id);
        let due = purchase.on.parse::<Date>().unwrap_or(month0).max(month0);
        items.push(Planned {
            wish_id: wish_id.to_owned(),
            label: wish.name.clone(),
            due,
            remaining: gap,
            on_card: purchase.source == WishSource::Card,
            created_at: wish.created_at.clone(),
            per_month: None,
            saved_this_month: false,
        });
        let (judged, _, _) = self.judge(alloc, month0, &items, wishes, settings);
        judged
            .first()
            .map(|m| {
                merge_cuts(
                    m.verdicts
                        .iter()
                        .filter(|v| v.wish_id.as_deref() == Some(wish_id) && !v.set_aside),
                )
            })
            .unwrap_or_default()
    }
}

/// The wishes screen, judged against one currency's allocation.
pub fn view_from(
    alloc: Option<&CurrencyAllocation>,
    plans: &[BudgetPlan],
    card_bill: Decimal,
    wishes: &[Wish],
    settings: &WishSettings,
    bills: &HashMap<String, BillNow>,
) -> WishesView {
    Inputs {
        alloc,
        plans: plans.to_vec(),
        card_bill,
        bills,
    }
    .view(wishes, settings)
}

fn sort_by_due(wishes: &mut [&Wish]) {
    wishes.sort_by(|a, b| (&a.due_on, &a.created_at).cmp(&(&b.due_on, &b.created_at)));
}

struct RowContext<'a> {
    wishes: &'a [Wish],
    months: &'a [WishMonth],
    families: &'a [Family],
    month0: &'a [Demand],
    bills: &'a HashMap<String, BillNow>,
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
            .filter(|d| d.wish_id.as_deref() == Some(wish_id) && d.set_aside)
            .map(|d| d.amount)
            .sum()
    }

    /// Why a planned wish with money still to find has no verdicts.
    fn why_unjudged(
        &self,
        wish: &Wish,
        children: &[WishRow],
        family: Option<&Family>,
    ) -> Option<&'static str> {
        let ids: Vec<&str> = std::iter::once(wish.wish_id.as_str())
            .chain(children.iter().map(|c| c.wish.wish_id.as_str()))
            .collect();
        let judged = self
            .months
            .iter()
            .flat_map(|m| &m.verdicts)
            .any(|v| v.wish_id.as_deref().is_some_and(|id| ids.contains(&id)));
        if judged {
            return None;
        }
        let unknown_bill = match &wish.kind {
            WishKind::BillChange { bill_id, .. } => match self.bills.get(bill_id) {
                Some(bill) if bill.amount == wish.amount => return None,
                Some(_) => false,
                None => true,
            },
            WishKind::Recurring { .. } => false,
            WishKind::OneOff => {
                let own = family.map_or(wish.amount, |f| f.rest) - wish.saved;
                let parts: Decimal = children
                    .iter()
                    .filter(|c| c.wish.status == WishStatus::Planned)
                    .map(|c| (c.wish.amount - c.wish.saved).max(Decimal::ZERO))
                    .sum();
                if own.max(Decimal::ZERO) + parts <= Decimal::ZERO {
                    return None;
                }
                false
            }
        };
        if self.months.is_empty() {
            return Some("no budget data");
        }
        if unknown_bill {
            return Some("bill can't be judged");
        }
        let last = self.months.last()?.month.parse::<Date>().ok()?;
        let due = wish.due_on.parse::<Date>().ok()?;
        (due.first_of_month() > last).then_some("too far ahead")
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
        let one_off = wish.kind == WishKind::OneOff;
        if wish.status == WishStatus::Planned {
            for v in self.verdicts(&wish.wish_id) {
                short += v.short + v.next_short;
                tradeoffs |= !v.cuts.is_empty() || v.on_card > Decimal::ZERO;
            }
            if one_off {
                monthly_set_aside = self.set_aside(&wish.wish_id);
            }
            for child in &children {
                short += child.short;
                tradeoffs |= child.tradeoffs;
                monthly_set_aside += child.monthly_set_aside;
            }
        }
        let not_judged = (wish.status == WishStatus::Planned)
            .then(|| self.why_unjudged(wish, &children, family))
            .flatten()
            .map(str::to_owned);
        WishRow {
            wish: wish.clone(),
            children,
            rest: family.map_or(Decimal::ZERO, |f| f.rest),
            over_cap: family.is_some_and(|f| f.over_cap),
            monthly_set_aside,
            short,
            tradeoffs,
            not_judged,
        }
    }
}

/// One cut per group, in first-cut order, keeping the first `from` and the
/// lowest `to`.
fn merge_cuts<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> Vec<Cut> {
    let mut out: Vec<Cut> = Vec::new();
    for cut in verdicts.flat_map(|v| &v.cuts) {
        match out.iter_mut().find(|c| c.group_id == cut.group_id) {
            Some(seen) => seen.to = seen.to.min(cut.to),
            None => out.push(cut.clone()),
        }
    }
    out
}

/// The cuts month 0 makes for one wish's own demands (not its set-asides).
pub fn month_zero_cuts(view: &WishesView, wish_id: &str) -> Vec<Cut> {
    view.months
        .first()
        .map(|month| {
            merge_cuts(
                month
                    .verdicts
                    .iter()
                    .filter(|v| v.wish_id.as_deref() == Some(wish_id) && !v.set_aside),
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::baseline::tests::{alloc, d, plan, plan_group, row};
    use super::*;
    use finance_types::gateway::{CardSettings, WishPurchase, WishSource, WishStatus};
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
            saved_month: None,
            status: WishStatus::Planned,
            bought: None,
            kind: WishKind::OneOff,
            must: false,
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
    fn bill_schedules_the_judging_knows() {
        assert_eq!(repeat_of("Weekly", 0), Some(Repeat::Weekly));
        assert_eq!(repeat_of("weekly", 1), Some(Repeat::Biweekly));
        assert_eq!(repeat_of("weekly", 2), None);
        assert_eq!(repeat_of("monthly", 0), Some(Repeat::Monthly));
        assert_eq!(repeat_of("monthly", 1), None);
        assert_eq!(repeat_of("yearly", 0), Some(Repeat::Yearly));
        assert_eq!(repeat_of("quarterly", 0), None);
    }

    #[test]
    fn default_month_inherits_ranking_from_stored_plans() {
        let a = alloc(
            "0",
            "0",
            vec![row("Going out", "120", "0"), row("Groceries", "450", "0")],
            vec![row("Going out", "120", "0"), row("Groceries", "450", "0")],
        );
        // The dashboard only carries October's unsaved default plan.
        let dashboard = [plan(
            "2026-10-01",
            vec![
                plan_group("going-out", "Going out", None, None),
                plan_group("groceries", "Groceries", None, None),
            ],
        )];
        let stored = [plan(
            "2026-09-01",
            vec![
                plan_group("g-out", "Going out", Some(2), None),
                plan_group("g-groc", "Groceries", Some(1), Some("300")),
            ],
        )];
        let s = no_card_room();
        let ws = [wish("tv", None, "200", "2026-10-20")];
        let alone = view_from(
            Some(&a),
            &dashboard,
            Decimal::ZERO,
            &ws,
            &s,
            &HashMap::new(),
        );
        assert!(alone.months[0].verdicts[0].cuts.is_empty());
        let plans = view_plans(dashboard.to_vec(), &stored);
        let view = view_from(Some(&a), &plans, Decimal::ZERO, &ws, &s, &HashMap::new());
        let cuts = &view.months[0].verdicts[0].cuts;
        assert_eq!(cuts[0].group_id, "going-out");
        assert_eq!((cuts[0].from, cuts[0].to), (d("120"), d("0")));
        assert_eq!(cuts[1].group_id, "groceries");
        assert_eq!(cuts[1].to, d("370"));
    }

    #[test]
    fn view_plans_prefer_the_dashboards_month() {
        let dash = plan("2026-10-01", vec![plan_group("a", "A", None, None)]);
        let stored = [
            plan("2026-10-01", vec![plan_group("old", "A", Some(1), None)]),
            plan("2026-09-01", vec![]),
        ];
        let plans = view_plans(vec![dash.clone()], &stored);
        assert_eq!(plans.len(), 2);
        assert!(plans.contains(&dash));
        assert!(plans.iter().any(|p| p.month == "2026-09-01"));
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
            &HashMap::new(),
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
    fn card_wish_next_month_cannot_pay_is_short() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut tv = wish("tv", None, "500", "2026-10-20");
        tv.source = WishSource::Card;
        let view = view_from(
            Some(&a),
            &[],
            Decimal::ZERO,
            &[tv],
            &WishSettings::default(),
            &HashMap::new(),
        );
        let charge = &view.months[1].verdicts[0];
        assert_eq!(charge.label, "Credit card");
        assert_eq!(charge.short, d("500"));
        assert_eq!(view.wishes[0].short, d("500"));
        assert!(view.wishes[0].tradeoffs);
    }

    fn no_card_room() -> WishSettings {
        WishSettings {
            cards: vec![CardSettings {
                limit: Decimal::ZERO,
                ..WishSettings::default().first()
            }],
        }
    }

    fn buy(amount: &str, on: &str, source: WishSource) -> WishPurchase {
        WishPurchase {
            amount: d(amount),
            on: on.into(),
            source,
            cards: Vec::new(),
        }
    }

    fn account() -> WishSource {
        WishSource::Account {
            account_id: "a".into(),
        }
    }

    #[test]
    fn early_buy_judges_the_unsaved_gap() {
        // 100 free covers the 100 already saved (earmarked).
        let a = alloc(
            "100",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        let mut tv = wish("tv", None, "900", "2026-12-20");
        tv.saved = d("100");
        let s = no_card_room();
        let wishes = [tv];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            plans: going_out_plan(),
            card_bill: Decimal::ZERO,
            bills: &bills,
        };
        // While planned, month 0 only sets aside a third of the gap, and a
        // set-aside's cuts are not a purchase's.
        let view = inputs.view(&wishes, &s);
        assert!(view.months[0].verdicts[0].set_aside);
        assert!(month_zero_cuts(&view, "tv").is_empty());
        let cuts = inputs.purchase_cuts(&wishes, &s, "tv", &buy("900", "2026-10-15", account()));
        assert_eq!(
            cuts,
            vec![Cut {
                group_id: "g-out".into(),
                name: "Going out".into(),
                from: d("1000"),
                to: d("200"),
            }]
        );
        // On the card, the gap goes on the card and nothing is cut.
        let s = WishSettings::default();
        let card = inputs.purchase_cuts(
            &wishes,
            &s,
            "tv",
            &buy("900", "2026-10-15", WishSource::Card),
        );
        assert!(card.is_empty());
        // Fully saved: nothing to judge.
        assert!(
            inputs
                .purchase_cuts(&wishes, &s, "tv", &buy("100", "2026-10-15", account()))
                .is_empty()
        );
    }

    #[test]
    fn unjudged_rows_say_why() {
        let a = alloc("1000", "1000", vec![], vec![]);
        let mut saved = wish("saved", None, "40", "2026-10-20");
        saved.saved = d("40");
        let mut change = wish("change", None, "45", "2026-11-01");
        change.kind = WishKind::BillChange {
            bill_id: "gone".into(),
            ends_on: None,
        };
        let ws = [
            wish("near", None, "50", "2026-10-20"),
            wish("far", None, "50", "2032-10-20"),
            saved,
            change,
        ];
        let view = view_from(
            Some(&a),
            &[],
            Decimal::ZERO,
            &ws,
            &WishSettings::default(),
            &HashMap::new(),
        );
        let why = |id: &str| {
            view.wishes
                .iter()
                .find(|r| r.wish.wish_id == id)
                .unwrap()
                .not_judged
                .clone()
        };
        assert_eq!(view.months.len(), 60);
        assert_eq!(why("near"), None);
        assert_eq!(why("far").as_deref(), Some("too far ahead"));
        assert_eq!(why("saved"), None);
        assert_eq!(why("change").as_deref(), Some("bill can't be judged"));
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
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Decimal::ZERO, &ws, &s, &HashMap::new());
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
            &HashMap::new(),
        );
        assert!(view.months.is_empty());
        assert_eq!(view.wishes.len(), 1);
        assert_eq!(view.wishes[0].not_judged.as_deref(), Some("no budget data"));
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
        assert!(!view.wishes[0].tradeoffs);
        assert_eq!(view.currency, Currency::eur());
    }

    #[test]
    fn month_zero_cuts_merge_repeated_groups() {
        let cut = |g: &str, from: &str, to: &str| Cut {
            group_id: g.into(),
            name: g.into(),
            from: d(from),
            to: d(to),
        };
        let verdict = |id: Option<&str>, cuts: Vec<Cut>| Verdict {
            wish_id: id.map(String::from),
            label: String::new(),
            amount: Decimal::ZERO,
            from_free: Decimal::ZERO,
            cuts,
            on_card: Decimal::ZERO,
            short: Decimal::ZERO,
            set_aside: false,
            next_short: Decimal::ZERO,
            next_cuts: Vec::new(),
            must: false,
            skipped: false,
        };
        let month = |verdicts| WishMonth {
            month: "2026-10-01".into(),
            free_before: Decimal::ZERO,
            free_left: Decimal::ZERO,
            card_charge_in: Decimal::ZERO,
            card_used: Decimal::ZERO,
            cards: Vec::new(),
            verdicts,
        };
        let view = WishesView {
            currency: Currency::eur(),
            settings: WishSettings::default(),
            card_stale: false,
            stale_cards: Vec::new(),
            changeable_bills: Vec::new(),
            months: vec![
                month(vec![
                    verdict(
                        Some("tv"),
                        vec![cut("g1", "120", "90"), cut("g2", "50", "40")],
                    ),
                    verdict(Some("other"), vec![cut("g3", "10", "0")]),
                    verdict(Some("tv"), vec![cut("g1", "90", "70")]),
                ]),
                month(vec![verdict(Some("tv"), vec![cut("g4", "5", "0")])]),
            ],
            wishes: vec![],
        };
        let cuts = month_zero_cuts(&view, "tv");
        assert_eq!(cuts, vec![cut("g1", "120", "70"), cut("g2", "50", "40")]);
        assert!(month_zero_cuts(&view, "none").is_empty());
    }
}
