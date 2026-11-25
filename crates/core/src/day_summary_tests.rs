use std::collections::HashMap;

use finance_types::report::Currency;
use finance_types::report::allocation::GroupAllocation;
use jiff::Zoned;
use rust_decimal::Decimal;

use super::*;
use crate::closure::tests::{account, snapshot};
use crate::model::Tx;
use crate::trends::tests::{mapping, out};

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// 25 November: six days left in the month, today included.
fn now() -> Zoned {
    "2026-11-25T21:00:00+01:00[Europe/Amsterdam]"
        .parse()
        .unwrap()
}

fn named(t: Tx, merchant: &str, memo: Option<&str>) -> Tx {
    Tx {
        counterparty_name: Some(merchant.into()),
        description: memo.map(str::to_owned),
        ..t
    }
}

fn snap() -> Snapshot {
    let today = "2026-11-25T12:00:00+01:00[Europe/Amsterdam]";
    let mut snap = snapshot(
        vec![account("firefly:1")],
        vec![
            named(
                out("d1", "18.50", Some("Thuisbezorgd"), today),
                "Thuisbezorgd",
                None,
            ),
            named(
                out("c1", "4.50", Some("Coffee"), today),
                "Cafe Blend",
                Some("BEA, Betaalpas CAFE BLEND,PAS1 NR:X, 25.11.26/08:40 AMSTERDAM"),
            ),
            named(out("g1", "30", Some("Jumbo"), today), "Jumbo", None),
            // Earlier this month, not today.
            named(
                out(
                    "d0",
                    "40",
                    Some("Thuisbezorgd"),
                    "2026-11-10T19:00:00+01:00[Europe/Amsterdam]",
                ),
                "Thuisbezorgd",
                None,
            ),
        ],
        &[],
    );
    snap.group_mappings = vec![
        mapping("Thuisbezorgd", "Food delivery"),
        mapping("Coffee", "Eating out"),
        mapping("Jumbo", "Groceries"),
    ];
    snap
}

fn group(name: &str, limit: &str, spent: &str) -> GroupAllocation {
    GroupAllocation {
        group: name.into(),
        limit: dec(limit),
        spent: dec(spent),
        remaining: Decimal::ZERO,
        projected_month_end: Decimal::ZERO,
    }
}

fn limits() -> Vec<(Currency, Vec<GroupAllocation>)> {
    vec![(
        Currency::eur(),
        vec![
            // 100 - (58.50 - 18.50 today) = 60 left / 6 days = 10 a day.
            group("Food delivery", "100", "58.50"),
            // 60 - 4.50 + 4.50 = 60 left / 6 = 10 a day.
            group("Eating out", "60", "4.50"),
            // 300 - 150 = 150 / 6 = 25 a day.
            group("Groceries", "300", "180"),
        ],
    )]
}

#[test]
fn an_over_day_names_what_pushed_it_over_and_what_could_be_skipped() {
    let s = day_summary(&limits(), &snap(), &now(), &HashMap::new());
    assert_eq!(s.date, "2026-11-25");
    assert!(s.over());
    let eur = &s.totals[0];
    assert_eq!(eur.spent, dec("53.00"));
    assert_eq!(eur.allowance, dec("45.00"));
    assert_eq!(eur.over_by, dec("8.00"));
    // Delivery (18.50 against 10) and groceries (30 against 25) went over.
    let over: Vec<&str> = eur.groups.iter().map(|g| g.group.as_str()).collect();
    assert_eq!(over, ["Food delivery", "Groceries"]);
    // Groceries are never avoidable; delivery and coffee are, largest first.
    let skipped: Vec<(&str, &str)> = s
        .avoidable
        .iter()
        .map(|p| (p.merchant.as_str(), p.instead.as_str()))
        .collect();
    assert_eq!(
        skipped,
        [
            (
                "Thuisbezorgd",
                "cooking at home, or picking it up on a walk"
            ),
            ("Cafe Blend", "making coffee at home"),
        ]
    );
    assert_eq!(s.avoidable[1].time.as_deref(), Some("08:40"));
    let (title, body) = push_text(&s);
    assert!(title.starts_with("Rafael: ") && title.contains("€8.00 over"));
    assert!(body.starts_with(
        "Thuisbezorgd €18.50: next time, try cooking at home, or picking it up on a walk."
    ));
    assert!(body.ends_with("— Rafael"));
    let lines = text_lines(&s);
    assert_eq!(
        lines[0],
        "Today: €53.00 spent against €45.00 the plan allows per day, €8.00 over."
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Cafe Blend at 08:40 €4.50"))
    );
}

#[test]
fn the_owners_ticks_override_the_defaults() {
    let mut ticks = HashMap::new();
    ticks.insert("Food delivery".to_owned(), false);
    ticks.insert("Groceries".to_owned(), true);
    let s = day_summary(&limits(), &snap(), &now(), &ticks);
    let merchants: Vec<&str> = s.avoidable.iter().map(|p| p.merchant.as_str()).collect();
    assert_eq!(merchants, ["Jumbo", "Cafe Blend"]);
    let groups = avoidable_groups(&snap(), &ticks);
    let row = |g: &str| groups.iter().find(|r| r.group == g).unwrap().clone();
    assert!(!row("Food delivery").avoidable && !row("Food delivery").default);
    assert!(row("Eating out").avoidable && row("Eating out").default);
    assert!(row("Groceries").avoidable);
}

#[test]
fn a_day_within_the_plan_is_not_over() {
    let roomy = vec![(
        Currency::eur(),
        vec![
            group("Food delivery", "500", "58.50"),
            group("Eating out", "500", "4.50"),
            group("Groceries", "900", "180"),
        ],
    )];
    let s = day_summary(&roomy, &snap(), &now(), &HashMap::new());
    assert!(!s.over());
    assert_eq!(s.avoidable.len(), 2, "still listed on Overview");
}

#[test]
fn defaults_follow_the_group_name() {
    assert!(default_avoidable("Eating out"));
    assert!(default_avoidable("Takeaway & delivery"));
    assert!(default_avoidable("Clothes"));
    assert!(default_avoidable("Going out"));
    assert!(!default_avoidable("Groceries"));
    assert!(!default_avoidable("Rent"));
    assert!(!default_avoidable("Health & fun"));
    assert!(!default_avoidable("Other"));
    assert_eq!(
        instead("Hobbies"),
        "a free plan: going out for a walk or to the park"
    );
    assert_eq!(instead("Other"), FALLBACK);
}

#[test]
fn ticks_are_saved_and_reset() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../storage/src/nudges.sql"))
        .unwrap();
    save_tick(&conn, "Coffee", Some(false)).unwrap();
    save_tick(&conn, "Coffee", Some(true)).unwrap();
    assert_eq!(load_ticks(&conn).unwrap().get("Coffee"), Some(&true));
    save_tick(&conn, "Coffee", None).unwrap();
    assert!(load_ticks(&conn).unwrap().is_empty());
}
