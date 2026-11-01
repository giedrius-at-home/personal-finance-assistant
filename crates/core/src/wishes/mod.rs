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
use rust_decimal::Decimal;
use schedule::{BillNow, Family, Planned, Saver};
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
    let (plans, card_bills, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        today: today(dashboard),
        plans,
        card_bills,
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
    let (plans, card_bills, bills) = dashboard_parts(dashboard, stored_plans, bills, settings);
    Inputs {
        alloc: allocation(dashboard),
        today: today(dashboard),
        plans,
        card_bills,
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

/// The owner's day the dashboard was built on.
fn today(dashboard: &Dashboard) -> Option<Date> {
    dashboard.generated_at.get(..10)?.parse().ok()
}

fn allocation(dashboard: &Dashboard) -> Option<&CurrencyAllocation> {
    dashboard.monthly_allocation.as_ref().and_then(|m| {
        m.currencies
            .iter()
            .find(|c| c.currency.code == "EUR")
            .or_else(|| m.currencies.first())
    })
}

/// Plans, each card's linked bill amount, and the bills the judging knows.
type DashboardParts = (
    Vec<BudgetPlan>,
    Vec<(String, Decimal)>,
    HashMap<String, BillNow>,
);

/// The budget plans, each card's bill amount and the bills as they stand.
fn dashboard_parts(
    dashboard: &Dashboard,
    stored_plans: &[BudgetPlan],
    bills: &[Bill],
    settings: &WishSettings,
) -> DashboardParts {
    let plans = view_plans(
        dashboard
            .monthly_budgets
            .iter()
            .map(|b| b.plan.clone())
            .collect(),
        stored_plans,
    );
    // Each card's bill is already counted in next month's fixed costs.
    let rows = &dashboard
        .next_month_subscriptions
        .as_ref()
        .unwrap_or(&dashboard.subscriptions)
        .rows;
    let card_bills = settings
        .cards
        .iter()
        .filter_map(|card| {
            let bill_id = card.bill_id.as_deref()?;
            let row = rows.iter().find(|row| row.bill_id == bill_id)?;
            Some((card.card_id.clone(), row.amount_max))
        })
        .collect();
    (plans, card_bills, bills_now(dashboard, bills))
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
    /// The day judged on, for which card statement is open; `None` reads
    /// the first of month 0, so month 0's statement is open and usage typed
    /// for the next statement counts as stale.
    pub today: Option<Date>,
    pub plans: Vec<BudgetPlan>,
    pub card_bills: Vec<(String, Decimal)>,
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
    ) -> (Vec<WishMonth>, Vec<String>) {
        let today = self.today.unwrap_or(month0);
        let months = schedule::horizon(items, month0);
        let (bases, stale_cards) = baseline::baselines(
            alloc,
            today,
            &self.plans,
            &self.card_bills,
            settings,
            schedule::earmarked(wishes),
            months,
        );
        let mut saver = Saver::new(items, month0, months);
        let mut repeats = schedule::repeat_demands(items, month0, months);
        // A closed statement takes no more card costs: the repeats that fell
        // on it are already in its usage (or the bill).
        if schedule::statement_month(today) > month0
            && let Some(first) = repeats.first_mut()
        {
            first.retain(|d| !d.on_card);
        }
        (ladder::judge(&bases, &repeats, &mut saver), stale_cards)
    }

    /// Card one-offs due before today go on the statement open today: a
    /// closed statement takes nothing more.
    fn open_statement(&self, items: &mut [Planned], month0: Date) {
        let today = self.today.unwrap_or(month0);
        for item in items
            .iter_mut()
            .filter(|i| i.on_card && i.per_month.is_none())
        {
            item.due = item.due.max(today);
        }
    }

    /// The wishes screen.
    pub fn view(&self, wishes: &[Wish], settings: &WishSettings) -> WishesView {
        let judged = self.alloc.and_then(|a| {
            let month0 = baseline::current_month(a)?;
            let (mut items, families) = schedule::plan_items(wishes, month0, self.bills);
            self.open_statement(&mut items, month0);
            let (judged, stale) = self.judge(a, month0, &items, wishes, settings);
            Some((judged, families, stale))
        });
        let currency = self.alloc.map_or_else(
            || {
                wishes
                    .first()
                    .map_or_else(Currency::eur, |w| w.currency.clone())
            },
            |a| a.currency.clone(),
        );
        let (months, families, stale_cards) = judged.unwrap_or_default();
        let ctx = RowContext {
            wishes,
            months: &months,
            families: &families,
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
        let must_keep = crate::closure::plan::wishes_must_keep(wishes, &currency.code)
            .iter()
            .map(|(_, v)| *v)
            .sum();
        WishesView {
            currency,
            settings: settings.clone(),
            card_stale: !stale_cards.is_empty(),
            stale_cards,
            wishes: tops.into_iter().map(|w| ctx.row(w)).collect(),
            months,
            changeable_bills,
            savings_must_keep: must_keep,
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
            // Already bought: month 0 sets nothing aside for it, even when
            // the card statement makes month 1 pay.
            saved_this_month: true,
            // A sub-wish of a must wish is a must too, as the view judges it.
            must: wish.must
                || wishes
                    .iter()
                    .any(|p| Some(&p.wish_id) == wish.parent_id.as_ref() && p.must),
        });
        self.open_statement(&mut items, month0);
        let landed = items
            .iter()
            .find(|i| i.wish_id == wish_id)
            .map_or(0, |i| i.months_from(month0));
        let (judged, _) = self.judge(alloc, month0, &items, wishes, settings);
        judged
            .get(landed)
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
    card_bills: Vec<(String, Decimal)>,
    wishes: &[Wish],
    settings: &WishSettings,
    bills: &HashMap<String, BillNow>,
) -> WishesView {
    Inputs {
        alloc,
        today: None,
        plans: plans.to_vec(),
        card_bills,
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
    bills: &'a HashMap<String, BillNow>,
}

impl RowContext<'_> {
    fn verdicts<'v>(&'v self, wish_id: &'v str) -> impl Iterator<Item = &'v Verdict> {
        self.months
            .iter()
            .flat_map(|m| &m.verdicts)
            .filter(move |v| v.wish_id.as_deref() == Some(wish_id))
    }

    /// What month 0 sets aside for `wish_id` (a skipped month sets nothing).
    fn set_aside(&self, wish_id: &str) -> Decimal {
        self.months
            .first()
            .into_iter()
            .flat_map(|m| &m.verdicts)
            .filter(|v| v.wish_id.as_deref() == Some(wish_id) && v.set_aside && !v.skipped)
            .map(|v| v.amount)
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
                tradeoffs |= !v.cuts.is_empty()
                    || v.on_card > Decimal::ZERO
                    || v.from_savings > Decimal::ZERO;
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
        let alone = view_from(Some(&a), &dashboard, Vec::new(), &ws, &s, &HashMap::new());
        assert!(alone.months[0].verdicts[0].cuts.is_empty());
        let plans = view_plans(dashboard.to_vec(), &stored);
        let view = view_from(Some(&a), &plans, Vec::new(), &ws, &s, &HashMap::new());
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
            Vec::new(),
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
            Vec::new(),
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

    #[test]
    fn card_wish_after_the_25th_is_next_statement() {
        let a = alloc("0", "1000", vec![], vec![]);
        let mut tv = wish("tv", None, "500", "2026-10-27");
        tv.source = WishSource::Card;
        let judge = |w: &Wish| {
            view_from(
                Some(&a),
                &[],
                Vec::new(),
                std::slice::from_ref(w),
                &WishSettings::default(),
                &HashMap::new(),
            )
        };
        let bought = |m: &WishMonth| {
            m.verdicts
                .iter()
                .find(|v| v.wish_id.as_deref() == Some("tv") && !v.set_aside)
                .cloned()
        };
        // 27 Oct is November's statement, charged in December.
        let view = judge(&tv);
        assert!(bought(&view.months[0]).is_none());
        assert_eq!(bought(&view.months[1]).unwrap().on_card, d("500"));
        assert_eq!(view.months[1].card_used, d("500"));
        assert_eq!(view.months[2].card_charge_in, d("500"));
        // From a bank account it is still October's.
        tv.source = account();
        let view = judge(&tv);
        assert_eq!(bought(&view.months[0]).unwrap().amount, d("500"));
        assert!(bought(&view.months[1]).is_none());
    }

    fn named_card(id: &str, used: &str, used_month: &str) -> CardSettings {
        CardSettings {
            card_id: id.into(),
            name: id.into(),
            used: d(used),
            used_month: used_month.into(),
            ..WishSettings::default().cards[0].clone()
        }
    }

    #[test]
    fn view_reports_stale_cards_and_per_card_payback() {
        let a = alloc("0", "0", vec![], vec![]);
        let mut tv = wish("w", None, "120", "2026-10-20");
        tv.source = WishSource::Card;
        let settings = WishSettings {
            cards: vec![
                named_card("visa", "50", "2026-10-01"),
                named_card("mc", "10", "2026-09-01"),
            ],
        };
        let view = view_from(Some(&a), &[], Vec::new(), &[tv], &settings, &HashMap::new());
        assert_eq!(view.stale_cards, vec!["mc".to_string()]);
        assert!(view.card_stale);
        assert_eq!(
            view.months[0].cards.iter().map(|c| c.used).sum::<Decimal>(),
            d("170")
        );
        assert_eq!(view.months[1].card_charge_in, d("170"));
    }

    fn no_card_room() -> WishSettings {
        WishSettings {
            cards: vec![CardSettings {
                limit: Decimal::ZERO,
                ..WishSettings::default().cards[0].clone()
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
            today: None,
            plans: going_out_plan(),
            card_bills: Vec::new(),
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
    fn closed_statement_takes_no_card_wishes() {
        // On 27 Oct October's statement is closed: an overdue card wish
        // goes on November's, and a card repeat that fell on October's is
        // already in its usage.
        let a = alloc("0", "1000", vec![], vec![]);
        let mut late = wish("late", None, "200", "2026-10-20");
        late.source = WishSource::Card;
        let mut gym = wish("gym", None, "30", "2026-10-10");
        gym.source = WishSource::Card;
        gym.kind = WishKind::Recurring {
            every: Repeat::Monthly,
            ends_on: None,
        };
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: Some("2026-10-27".parse().unwrap()),
            plans: Vec::new(),
            card_bills: Vec::new(),
            bills: &bills,
        };
        let view = inputs.view(&[late, gym], &WishSettings::default());
        let paid = |m: usize, id: &str| {
            view.months[m]
                .verdicts
                .iter()
                .any(|v| v.wish_id.as_deref() == Some(id) && !v.set_aside)
        };
        assert!(!paid(0, "late") && !paid(0, "gym"));
        assert!(paid(1, "late") && paid(1, "gym"));
        assert_eq!(view.months[1].card_used, d("230"));
    }

    #[test]
    fn card_buy_after_the_25th_records_next_months_cuts() {
        // Bought on the card on 28 Oct: November's statement, so November
        // (month 1) pays for it, and with no card room it cuts going out.
        let a = alloc(
            "0",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        let wishes = [wish("tv", None, "900", "2026-12-20")];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: Some("2026-10-28".parse().unwrap()),
            plans: going_out_plan(),
            card_bills: Vec::new(),
            bills: &bills,
        };
        let cuts = inputs.purchase_cuts(
            &wishes,
            &no_card_room(),
            "tv",
            &buy("900", "2026-10-28", WishSource::Card),
        );
        assert_eq!(
            cuts,
            vec![Cut {
                group_id: "g-out".into(),
                name: "Going out".into(),
                from: d("1000"),
                to: d("100"),
            }]
        );
    }

    #[test]
    fn buying_a_sub_of_a_must_wish_is_judged_as_must() {
        let a = alloc(
            "250",
            "0",
            vec![row("Going out", "1000", "0")],
            vec![row("Going out", "1000", "0")],
        );
        // The must hotel's set-aside (100) comes first; the optional tv keeps
        // all 600 from the 150 free left plus a 450 cut to going out (set-asides
        // cut groups before they skip, and never use the cards).
        let tv = wish("tv", None, "600", "2026-12-20");
        let mut trip = wish("trip", None, "100", "2026-12-31");
        trip.must = true;
        let hotel = wish("hotel", Some("trip"), "100", "2026-12-31");
        let wishes = [tv, trip, hotel];
        let bills = HashMap::new();
        let inputs = Inputs {
            alloc: Some(&a),
            today: None,
            plans: going_out_plan(),
            card_bills: Vec::new(),
            bills: &bills,
        };
        let view = inputs.view(&wishes, &no_card_room());
        let tv: Vec<_> = view.months[0]
            .verdicts
            .iter()
            .filter(|v| v.wish_id.as_deref() == Some("tv"))
            .map(|v| (v.set_aside, v.skipped, v.amount, v.from_free, v.cuts.len()))
            .collect();
        assert_eq!(tv, vec![(true, false, d("600"), d("150"), 1)]);
        // The must purchase is judged before the optional set-aside, so the
        // 250 free covers it and nothing is cut.
        let cuts = inputs.purchase_cuts(
            &wishes,
            &no_card_room(),
            "hotel",
            &buy("100", "2026-10-15", account()),
        );
        assert!(cuts.is_empty(), "{cuts:?}");
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
            Vec::new(),
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
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
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
        // Must-wishes addendum: an optional set-aside that does not fit
        // without the cards is skipped, so a month with nothing free sets
        // nothing aside (it would be hotel 400/2, rest 600/3).
        assert_eq!(trip.children[0].monthly_set_aside, Decimal::ZERO);
        assert_eq!(trip.monthly_set_aside, Decimal::ZERO);
        assert_eq!(view.wishes[1].monthly_set_aside, Decimal::ZERO);
    }

    #[test]
    fn wish_paid_from_savings_is_a_tradeoff() {
        let mut a = alloc("0", "0", vec![], vec![]);
        a.savings_balance = d("500");
        let ws = [wish("tv", None, "150", "2026-10-20")];
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
        let v = &view.months[0].verdicts[0];
        assert_eq!(v.from_savings, d("150"));
        assert!(view.wishes[0].tradeoffs);
        assert_eq!(view.wishes[0].short, Decimal::ZERO);
    }

    #[test]
    fn parent_row_sums_month_zero_set_asides_that_fit() {
        let a = alloc("1000", "1000", vec![], vec![]);
        let ws = [
            wish("trip", None, "1000", "2026-12-31"),
            wish("hotel", Some("trip"), "400", "2026-11-10"),
            wish("later", None, "60", "2026-12-01"),
        ];
        let s = no_card_room();
        let view = view_from(Some(&a), &[], Vec::new(), &ws, &s, &HashMap::new());
        let trip = view
            .wishes
            .iter()
            .find(|r| r.wish.wish_id == "trip")
            .unwrap();
        // Month 0 set-asides, soonest first: hotel (10 Nov) keeps all 400,
        // "later" (1 Dec) its 60, the trip's rest (31 Dec) the 540 left.
        assert_eq!(trip.children[0].monthly_set_aside, d("400"));
        assert_eq!(trip.monthly_set_aside, d("940"));
        assert_eq!(trip.short, Decimal::ZERO);
    }

    #[test]
    fn no_allocation_leaves_wishes_unjudged() {
        let view = view_from(
            None,
            &[],
            Vec::new(),
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
            from_savings: Decimal::ZERO,
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
            savings_before: Decimal::ZERO,
            savings_left: Decimal::ZERO,
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
            savings_must_keep: Decimal::ZERO,
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
