//! Development check: the vocabulary version and prompt version the
//! suggestion runner computes for a database, to compare with the
//! `prompt_version` of the proposals the Python service stored, plus what
//! the next pass would pick up.

use std::collections::HashMap;
use std::time::Instant;

use finance_core::ai::{history, runner};
use finance_core::{Snapshot, review};
use finance_storage::Store;
use finance_types::ai::{AiSettings, Classifier};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or(finance_storage::default_db_path()?);
    let store = Store::open_read_only(&path)?;
    let snap = Snapshot::load(&store)?;
    let started = Instant::now();
    let h = history::build(&snap.transactions);
    eprintln!(
        "history in {:?}: {} categories, {} rules, {} examples",
        started.elapsed(),
        h.categories.len(),
        h.rules.len(),
        h.examples.len()
    );
    println!("vocabulary_version {}", h.version);
    // DUMP_HISTORY=<out.json> [QUERIES=<python dump with "retrieval">]
    if let Some(out) = std::env::var_os("DUMP_HISTORY") {
        let queries: Vec<String> = std::env::var_os("QUERIES")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| {
                v["retrieval"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
            })
            .unwrap_or_default();
        let txs: serde_json::Map<String, serde_json::Value> = snap
            .transactions
            .iter()
            .map(|t| {
                (
                    t.ledger_id.to_string(),
                    serde_json::json!([t.category, t.description, t.kind.as_str()]),
                )
            })
            .collect();
        std::fs::write(
            out,
            serde_json::to_string(&serde_json::json!({
                "categories": h.categories,
                "rules": h.rules,
                "examples": h.examples,
                "txs": txs,
                "retrieval": finance_core::ai::retrieval::lexical(&queries, &h.retrieval),
            }))?,
        )?;
    }
    for (classifier, embedding) in [
        (Classifier::TypeSafe, ""),
        (Classifier::TypeSafe, "qwen3-embedding:0.6b"),
        (Classifier::TypeSafe, "embeddinggemma"),
        (Classifier::Ollama, ""),
    ] {
        let settings = AiSettings {
            classifier,
            embedding_model: embedding.into(),
            ..AiSettings::default()
        };
        println!(
            "{classifier:?} {embedding:?} {}",
            runner::prompt_version(&settings, &h)
        );
    }
    let conn = store.connection();
    let inputs = runner::Inputs {
        transactions: snap.transactions.clone(),
        pending: runner::list_pending(conn)?,
        latest: review::latest_proposals(conn)?,
        daily_open: snap
            .daily_review_entries
            .iter()
            .filter(|e| e.resolved_at.is_none())
            .map(|e| e.transaction_id.clone())
            .collect(),
        deferred: snap.deferred_sources.clone(),
    };
    for classifier in [Classifier::TypeSafe, Classifier::Ollama] {
        let settings = AiSettings {
            classifier,
            ..AiSettings::default()
        };
        let version = runner::prompt_version(&settings, &h);
        let plan = runner::plan(
            &inputs,
            settings.classifier_model(),
            &version,
            1000,
            &HashMap::new(),
            Instant::now(),
        );
        println!(
            "{classifier:?}: {} candidates, {} to supersede, {} replacing pending",
            plan.candidates.len(),
            plan.supersede.len(),
            plan.retry.len()
        );
    }
    Ok(())
}
