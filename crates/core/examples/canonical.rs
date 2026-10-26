//! Development check: print the canonical transactions as JSON lines, in the
//! shape of the Python `canonical_records` payloads, for a parity diff.

use finance_core::Snapshot;
use finance_storage::Store;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let store = Store::open_read_only(&path)?;
    let snapshot = Snapshot::load(&store)?;
    for t in &snapshot.raw_transactions {
        println!(
            "{}",
            serde_json::json!({
                "transaction_id": t.transaction_id,
                "account_id": t.account_id,
                "counterparty_account_id": t.counterparty_account_id,
                "kind": t.kind.as_str(),
                "amount": t.amount.normalize().to_string(),
                "currency": t.currency.code,
                "occurred_at": t.occurred_at.timestamp().to_string(),
                "purchased_on": t.purchased_on.map(|d| d.to_string()),
                "description": t.description,
                "counterparty_name": t.counterparty_name,
                "category": t.category,
                "subscription_id": t.subscription_id,
                "source_country": t.source_country.as_ref().map(|c| &c.code),
                "destination_country": t.destination_country.as_ref().map(|c| &c.code),
                "transfer_evidence": t.transfer_evidence,
                "firefly_group_id": t.group_id,
            })
        );
    }
    for a in &snapshot.accounts {
        eprintln!(
            "{} {} {} {}",
            a.account_id, a.name, a.balance.amount, a.include_net_worth
        );
    }
    Ok(())
}
