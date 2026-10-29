//! Monthly budget plans (Python `web.py` `/monthly-budget/save`,
//! `SQLiteDerivedState.save_budget_plan`).

use std::collections::HashSet;

use finance_types::gateway::BudgetPlan;
use rusqlite::params;

use super::{Writer, invalid};
use crate::groups::{DEFAULT_GROUPS, expense_categories};
use crate::model::Kind;
use crate::{Error, Result, Snapshot};

/// Groups the site always offers, besides the reviewed and custom ones.
const EXTRA_GROUPS: [&str; 5] = ["Coffee", "Drinks", "Takeaway", "Delivery", "Snacks"];

/// Save `plan` if nobody saved this month since `plan.revision`; returns the
/// stored plan with its new revision.
pub fn save_budget_plan(w: &Writer<'_>, snap: &Snapshot, plan: BudgetPlan) -> Result<BudgetPlan> {
    let currencies: HashSet<&str> = snap
        .transactions
        .iter()
        .map(|t| t.currency.code.as_str())
        .chain(snap.accounts.iter().map(|a| a.currency().code.as_str()))
        .collect();
    if !currencies.contains(plan.currency.code.as_str()) {
        return Err(invalid("invalid budget or overlapping groups"));
    }
    if !plan.month.ends_with("-01") || plan.groups.len() > 40 {
        return Err(invalid("invalid budget or overlapping groups"));
    }
    let categories = expense_categories(&snap.transactions);
    let mut groups: HashSet<String> = DEFAULT_GROUPS.iter().map(|g| (*g).to_owned()).collect();
    groups.extend(snap.custom_groups.iter().cloned());
    groups.extend(snap.group_mappings.iter().filter_map(|m| m.group.clone()));
    groups.extend(EXTRA_GROUPS.iter().map(|g| (*g).to_owned()));
    let purchases: HashSet<&str> = snap
        .transactions
        .iter()
        .filter(|t| t.currency == plan.currency && matches!(t.kind, Kind::Outflow | Kind::Refund))
        .map(|t| t.transaction_id.as_str())
        .collect();
    let mut seen_names = HashSet::new();
    for group in &plan.groups {
        let valid = group.categories.iter().all(|c| categories.contains(c))
            && group.source_groups.iter().all(|g| groups.contains(g))
            && group
                .transaction_ids
                .iter()
                .all(|id| purchases.contains(id.as_str()))
            && !group.name.trim().is_empty()
            && group.amount >= rust_decimal::Decimal::ZERO
            && group.importance != Some(0)
            && group.floor.is_none_or(|f| f >= rust_decimal::Decimal::ZERO)
            && seen_names.insert(group.name.trim().to_lowercase());
        if !valid {
            return Err(invalid("invalid budget or overlapping groups"));
        }
    }
    let saved = BudgetPlan {
        revision: plan.revision + 1,
        ..plan.clone()
    };
    let json = serde_json::to_string(&saved).map_err(|e| Error::Invalid(e.to_string()))?;
    let changed = w.tx.execute(
        "INSERT INTO monthly_budget_plans (month, currency_code, revision, updated_at, payload_json) \
         SELECT ?1, ?2, ?3, ?4, ?5 WHERE COALESCE((SELECT revision FROM monthly_budget_plans \
         WHERE month = ?1 AND currency_code = ?2), 0) = ?6 \
         ON CONFLICT(month, currency_code) DO UPDATE SET revision = excluded.revision, \
         updated_at = excluded.updated_at, payload_json = excluded.payload_json",
        params![
            saved.month,
            saved.currency.code,
            saved.revision,
            w.stamp(),
            json,
            plan.revision
        ],
    )?;
    if changed != 1 {
        return Err(invalid("budget changed; refresh"));
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::fixture::db;
    use finance_types::gateway::BudgetGroup;
    use finance_types::report::Currency;
    use rust_decimal::Decimal;

    fn plan(importance: Option<u8>, floor: Option<i64>) -> BudgetPlan {
        BudgetPlan {
            month: "2026-10-01".into(),
            currency: Currency::new("EUR"),
            revision: 0,
            groups: vec![BudgetGroup {
                id: "going-out".into(),
                name: "Going out".into(),
                amount: Decimal::from(120),
                source_groups: Vec::new(),
                categories: Vec::new(),
                transaction_ids: Vec::new(),
                account_id: None,
                importance,
                floor: floor.map(Decimal::from),
            }],
        }
    }

    #[test]
    fn rejects_zero_importance_and_negative_floor() {
        let mut db = db();
        for bad in [plan(Some(0), None), plan(None, Some(-1))] {
            let err = db
                .write(|w, snap| save_budget_plan(w, snap, bad))
                .unwrap_err();
            assert_eq!(err.to_string(), "invalid budget or overlapping groups");
        }
        let ok = db
            .write(|w, snap| save_budget_plan(w, snap, plan(Some(3), Some(0))))
            .unwrap();
        assert_eq!(ok.revision, 1);
    }
}
