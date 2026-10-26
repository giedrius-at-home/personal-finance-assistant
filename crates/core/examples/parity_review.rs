//! Development check: print the review inbox, the daily review and a few
//! transaction searches as JSON, keyed like the Python golden report.

use finance_core::{Ctx, Snapshot, review};
use finance_storage::Store;
use finance_types::report::review::TransactionSearchQuery;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let store = Store::open_read_only(&path)?;
    let started = std::time::Instant::now();
    let snap = Snapshot::load(&store)?;
    eprintln!("snapshot loaded in {:?}", started.elapsed());
    let ctx = Ctx::new(
        &snap,
        &"2026-10-26T21:00:00+01:00[Europe/Amsterdam]".parse()?,
    );
    let search = |query: serde_json::Value| -> anyhow::Result<serde_json::Value> {
        let query: TransactionSearchQuery = serde_json::from_value(query)?;
        Ok(serde_json::to_value(review::search_transactions(
            &ctx, &query,
        )?)?)
    };
    let out = json!({
        "review_items": review::review_items(&ctx),
        "daily_review": review::daily_inbox(&ctx, false),
        "search_albert": search(json!({"q": "albert"}))?,
        "search_revolut": search(json!({"q": "revolut", "page": 2}))?,
        "search_groceries": search(json!({"q": "groceries", "page_size": 200}))?,
        "search_amount": search(json!({"min": "100", "max": "100", "page_size": 200}))?,
        "search_out_october": search(json!({
            "direction": "out", "from": "2026-10-01", "to": "2026-10-26", "page_size": 200
        }))?,
        "search_transfers": search(json!({"direction": "transfer", "page_size": 200}))?,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
