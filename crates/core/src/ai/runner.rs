//! The suggestion pass (Python `CategorySuggestionRunner`): pick up to a
//! batch of uncategorised transactions that need a (new) proposal, retrieve
//! similar ledger examples, ask the classifier, and store reviewable
//! proposals. It never changes a category; [`super::daily`] may, later.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use finance_types::ai::{AiSettings, Classifier};
use finance_types::report::review::{CategoryProposal, CheckStatus, ProposalStatus};
use rusqlite::{Connection, params};

use super::categorise::{self, POLICY_FINGERPRINT, Qwen, Stamp, TypeSafe};
use super::check::{self, Checker, MerchantIndex};
use super::history::{self, History};
use super::retrieval::{self, EmbeddingCache, Example};
use super::text::{has_merchant_description, normalize_merchant};
use super::typesafe;
use super::{Categoriser, Choose, Embed, Generate};
use crate::model::{Kind, Tx};
use crate::review::latest_proposals;
use crate::service::Service;
use crate::writes::review::{get_pending, record_proposal};
use crate::writes::transactions::{iso_now, sha256_hex};
use crate::writes::{Writer, utc_stamp};
use crate::{Error, Result};

/// A failed or unavailable proposal is retried after five minutes.
pub const RETRY_AFTER: Duration = Duration::from_secs(300);
/// At most this many batches per pass.
pub const MAX_BATCHES: usize = 10;
const DAILY_PLACEHOLDER: &str = "daily-review-v1";

/// The models one pass may use.
#[derive(Clone, Copy)]
pub struct Backends<'a> {
    pub settings: &'a AiSettings,
    /// The Ollama classifier model (and second reviewer).
    pub generate: Option<&'a dyn Generate>,
    /// The Ollama embedding model, when one is configured.
    pub embed: Option<&'a dyn Embed>,
    /// TypeSafe, when it is the classifier.
    pub choose: Option<&'a dyn Choose>,
}

impl std::fmt::Debug for Backends<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends")
            .field("classifier", &self.settings.classifier)
            .finish_non_exhaustive()
    }
}

/// `category-v4-…`: changes whenever the classifier prompt, the ledger
/// vocabulary, the retrieval set-up or the review policy changes, so older
/// proposals are regenerated.
pub fn prompt_version(settings: &AiSettings, history: &History) -> String {
    let classifier = if settings.classifier == Classifier::TypeSafe {
        typesafe::PROMPT_VERSION
    } else {
        categorise::PROMPT_VERSION
    };
    let material = format!(
        "{classifier}|{}|hybrid-ledger-context-v2|{}|geo2|{POLICY_FINGERPRINT}",
        history.version,
        settings.embedding().unwrap_or("text-only")
    );
    format!("category-v4-{}", &sha256_hex(&material)[..16])
}

/// What one batch reads.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub transactions: Vec<Tx>,
    /// The oldest 500 pending proposals.
    pub pending: Vec<CategoryProposal>,
    /// The latest non-superseded proposal of each source.
    pub latest: Vec<CategoryProposal>,
    /// Transaction ids (`firefly:<id>`) with an open daily review entry.
    pub daily_open: HashSet<String>,
    /// Source keys the owner set aside ("No idea").
    pub deferred: HashSet<String>,
}

/// What one batch will do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Pending proposals that can no longer apply.
    pub supersede: Vec<String>,
    /// Ledger ids to propose for, in order.
    pub candidates: Vec<i64>,
    /// Ledger id → the pending proposal a new one replaces.
    pub retry: HashMap<i64, String>,
}

fn ledger_of(p: &CategoryProposal) -> Option<i64> {
    (p.source.system == "firefly_iii" && p.source.entity == "transaction")
        .then(|| p.source.external_id.parse().ok())
        .flatten()
}

/// The selection half of `_run_batch`.
pub fn plan(
    inputs: &Inputs,
    model: &str,
    prompt_version: &str,
    batch_size: usize,
    retry_after: &HashMap<String, Instant>,
    now: Instant,
) -> Plan {
    let due = |id: &str| retry_after.get(id).is_none_or(|t| now >= *t);
    let current: HashMap<i64, &Tx> = inputs
        .transactions
        .iter()
        .map(|t| (t.ledger_id, t))
        .collect();
    let latest: HashMap<i64, &CategoryProposal> = inputs
        .latest
        .iter()
        .filter_map(|p| ledger_of(p).map(|id| (id, p)))
        .collect();
    let needs = |t: &Tx| {
        t.kind != Kind::Transfer
            && t.category.is_none()
            && has_merchant_description(t.description.as_deref())
    };
    let mut plan = Plan::default();
    let mut regenerate: HashSet<i64> = HashSet::new();
    // A classifier or prompt change is a bounded backfill: regenerate the
    // latest decision of every uncategorised transaction made otherwise.
    for t in inputs.transactions.iter().filter(|t| needs(t)) {
        let Some(previous) = latest.get(&t.ledger_id) else {
            continue;
        };
        if previous.model_version.as_deref() == Some(model)
            && previous.prompt_version.as_deref() == Some(prompt_version)
        {
            continue;
        }
        regenerate.insert(t.ledger_id);
        if previous.status == ProposalStatus::Pending {
            plan.retry.insert(t.ledger_id, previous.proposal_id.clone());
        }
    }
    let mut pending: Vec<&CategoryProposal> = inputs.pending.iter().collect();
    let mut seen: HashSet<&str> = pending.iter().map(|p| p.proposal_id.as_str()).collect();
    for t in &inputs.transactions {
        if inputs.daily_open.contains(&t.transaction_id)
            && let Some(p) = latest.get(&t.ledger_id)
            && p.status == ProposalStatus::Pending
            && seen.insert(p.proposal_id.as_str())
        {
            pending.push(p);
        }
    }
    for proposal in pending {
        let ledger = ledger_of(proposal);
        let t = ledger.and_then(|id| current.get(&id));
        let Some(t) = t else {
            if !current.is_empty() {
                plan.supersede.push(proposal.proposal_id.clone());
            }
            continue;
        };
        if t.category.is_some() {
            plan.supersede.push(proposal.proposal_id.clone());
            continue;
        }
        let retry = if proposal.prompt_version.as_deref() == Some(DAILY_PLACEHOLDER) {
            proposal.reasons.iter().any(|r| r == "awaiting_model")
                || (proposal
                    .reasons
                    .iter()
                    .any(|r| r == "missing_bank_description")
                    && has_merchant_description(t.description.as_deref()))
        } else if proposal.model_version.as_deref() != Some(model)
            || proposal.prompt_version.as_deref() != Some(prompt_version)
        {
            due(&proposal.proposal_id)
        } else {
            (proposal.sanity_check.status == CheckStatus::Unavailable
                || (proposal.proposed_category.is_none()
                    && proposal.reasons.iter().any(|r| r == "model_unavailable")))
                && due(&proposal.proposal_id)
        };
        if retry {
            regenerate.insert(t.ledger_id);
            plan.retry.insert(t.ledger_id, proposal.proposal_id.clone());
        }
    }
    let mut ordered: Vec<&Tx> = inputs
        .transactions
        .iter()
        .filter(|t| {
            !inputs.deferred.contains(&t.source_key())
                && needs(t)
                && (regenerate.contains(&t.ledger_id) || !latest.contains_key(&t.ledger_id))
        })
        .collect();
    ordered.sort_by(|a, b| {
        let key = |t: &Tx| {
            (
                inputs.daily_open.contains(&t.transaction_id),
                t.occurred_at.timestamp(),
            )
        };
        key(b).cmp(&key(a))
    });
    plan.candidates = ordered
        .into_iter()
        .take(batch_size)
        .map(|t| t.ledger_id)
        .collect();
    plan
}

/// `resolve_proposal(…, SUPERSEDED)`.
fn supersede(w: &Writer<'_>, id: &str) -> Result<bool> {
    Ok(w.tx.execute(
        "UPDATE category_proposals SET status = 'superseded', updated_at = ?1 \
         WHERE proposal_id = ?2 AND status = 'pending'",
        params![iso_now(w), id],
    )? == 1)
}

/// Store a new proposal and retire the one it replaces when it is a real
/// answer (a category, an explicit abstention, or a daily placeholder).
/// Returns whether `previous` was superseded.
pub fn record(w: &Writer<'_>, proposal: &CategoryProposal, previous: Option<&str>) -> Result<bool> {
    record_proposal(w, proposal)?;
    let Some(previous) = previous.filter(|p| *p != proposal.proposal_id) else {
        return Ok(false);
    };
    let placeholder = get_pending(w, previous)?
        .is_some_and(|(old, _)| old.prompt_version.as_deref() == Some(DAILY_PLACEHOLDER));
    if proposal.proposed_category.is_some()
        || proposal.reasons.iter().any(|r| r == "model_abstained")
        || placeholder
    {
        return supersede(w, previous);
    }
    Ok(false)
}

/// The oldest 500 pending proposals (`list_pending`).
pub fn list_pending(conn: &Connection) -> Result<Vec<CategoryProposal>> {
    let mut stmt = conn.prepare(
        "SELECT payload_json FROM category_proposals WHERE status = 'pending' \
         ORDER BY created_at LIMIT 500",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|p| {
            serde_json::from_str(p).map_err(|e| Error::Invalid(format!("category proposal: {e}")))
        })
        .collect()
}

/// Where a pass reads and writes: the app's [`Service`], or a test database.
pub trait Store {
    fn inputs(&self) -> std::result::Result<Inputs, String>;
    fn write(
        &self,
        f: &mut dyn FnMut(&Writer<'_>) -> Result<()>,
    ) -> std::result::Result<(), String>;
    /// Let the screens see the batch.
    fn done(&self);
}

impl Store for Service {
    fn inputs(&self) -> std::result::Result<Inputs, String> {
        let mut inputs = self.read(|_, snap| {
            Ok(Inputs {
                transactions: snap.transactions.clone(),
                daily_open: snap
                    .daily_review_entries
                    .iter()
                    .filter(|e| e.resolved_at.is_none())
                    .map(|e| e.transaction_id.clone())
                    .collect(),
                deferred: snap.deferred_sources.clone(),
                ..Inputs::default()
            })
        })?;
        let (pending, latest) =
            self.read_store(|conn| Ok((list_pending(conn)?, latest_proposals(conn)?)))?;
        inputs.pending = pending;
        inputs.latest = latest;
        Ok(inputs)
    }

    fn write(
        &self,
        f: &mut dyn FnMut(&Writer<'_>) -> Result<()>,
    ) -> std::result::Result<(), String> {
        self.write_raw(|w| f(w))
    }

    fn done(&self) {
        self.refresh();
    }
}

/// The runner's memory across passes: retry times and embeddings.
#[derive(Debug, Default)]
pub struct Runner {
    retry_after: HashMap<String, Instant>,
    embeddings: EmbeddingCache,
}

impl Runner {
    /// Up to [`MAX_BATCHES`] batches; the number of transactions proposed for.
    pub fn run(
        &mut self,
        store: &dyn Store,
        backends: Backends<'_>,
    ) -> std::result::Result<usize, String> {
        let size = backends.settings.batch_size.max(1) as usize;
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let count = self.batch(store, backends)?;
            total += count;
            if count < size {
                break;
            }
        }
        Ok(total)
    }

    fn batch(
        &mut self,
        store: &dyn Store,
        backends: Backends<'_>,
    ) -> std::result::Result<usize, String> {
        let settings = backends.settings;
        let typesafe = settings.classifier == Classifier::TypeSafe;
        let qwen = backends.generate.map(|backend| Qwen { backend });
        let jev = backends.choose.map(|client| TypeSafe { client });
        let ollama_check = backends.generate.map(|backend| check::Ollama { backend });
        let jev_check = backends.choose.map(|client| check::TypeSafe { client });
        let (categoriser, checker): (&dyn Categoriser, Option<&dyn Checker>) =
            match (settings.classifier, &qwen, &jev) {
                (Classifier::TypeSafe, _, Some(jev)) => {
                    (jev, jev_check.as_ref().map(|c| c as &dyn Checker))
                }
                (Classifier::Ollama, Some(qwen), _) => {
                    (qwen, ollama_check.as_ref().map(|c| c as &dyn Checker))
                }
                _ => return Ok(0),
            };
        let inputs = store.inputs()?;
        let history = history::build(&inputs.transactions);
        let prompt_version = prompt_version(settings, &history);
        let model = settings.classifier_model().to_owned();
        let plan = plan(
            &inputs,
            &model,
            &prompt_version,
            settings.batch_size.max(1) as usize,
            &self.retry_after,
            Instant::now(),
        );
        if !plan.supersede.is_empty() {
            store.write(&mut |w| {
                for id in &plan.supersede {
                    supersede(w, id)?;
                }
                Ok(())
            })?;
        }
        if plan.candidates.is_empty() {
            if !plan.supersede.is_empty() {
                store.done();
            }
            return Ok(0);
        }
        let by_id: HashMap<i64, &Tx> = inputs
            .transactions
            .iter()
            .map(|t| (t.ledger_id, t))
            .collect();
        let candidates: Vec<(&Tx, String)> = plan
            .candidates
            .iter()
            .filter_map(|id| by_id.get(id))
            .map(|t| {
                (
                    *t,
                    normalize_merchant(t.description.as_deref().unwrap_or("")),
                )
            })
            .collect();
        let queries: Vec<String> = candidates.iter().map(|(_, m)| m.clone()).collect();
        let mut retrieved = retrieval::lexical(&queries, &history.retrieval);
        if let (Some(model), Some(embedder)) = (settings.embedding(), backends.embed)
            && !history.retrieval.is_empty()
            && let Ok(semantic) = retrieval::semantic(
                embedder,
                model,
                &queries,
                &history.retrieval,
                &mut self.embeddings,
            )
        {
            // An unavailable embedding model must not erase text matches.
            for (query, examples) in &mut retrieved {
                *examples =
                    retrieval::merge(examples, semantic.get(query).map_or(&[], Vec::as_slice));
            }
        }
        let index = MerchantIndex::build(&inputs.transactions);
        let stamp_now = utc_stamp(jiff::Timestamp::now());
        let stamp = Stamp {
            model_version: &model,
            prompt_version: &prompt_version,
            baseline: !typesafe,
            now: &stamp_now,
        };
        let empty: Vec<Example> = Vec::new();
        for (t, merchant) in &candidates {
            let found = retrieved.get(merchant).unwrap_or(&empty);
            let examples = if !typesafe && found.is_empty() {
                &history.examples
            } else {
                found
            };
            let Some(proposal) =
                categorise::propose(&stamp, &history, categoriser, checker, &index, t, examples)
            else {
                continue;
            };
            let previous = plan.retry.get(&t.ledger_id).cloned();
            let mut superseded = false;
            store.write(&mut |w| {
                superseded = record(w, &proposal, previous.as_deref())?;
                Ok(())
            })?;
            let later = Instant::now() + RETRY_AFTER;
            if let Some(previous) = previous {
                if superseded {
                    self.retry_after.remove(&previous);
                } else {
                    self.retry_after.insert(previous, later);
                }
            }
            if proposal.reasons.iter().any(|r| r == "model_unavailable")
                || proposal.sanity_check.status == CheckStatus::Unavailable
            {
                self.retry_after.insert(proposal.proposal_id.clone(), later);
            }
        }
        store.done();
        Ok(candidates.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::BackendError;
    use crate::ai::history::tests::tx;
    use crate::writes::fixture::{self, Db};
    use std::cell::{Cell, RefCell};

    fn proposal(
        id: &str,
        ledger: i64,
        model: Option<&str>,
        prompt: &str,
        reasons: &[&str],
    ) -> CategoryProposal {
        let mut p: CategoryProposal = serde_json::from_value(serde_json::json!({
            "proposal_id": id,
            "source": {"system": "firefly_iii", "entity": "transaction", "external_id": ledger.to_string()},
            "method": "abstain", "policy_version": "p", "vocabulary_version": "v",
            "feature_hash": "f", "normalized_merchant": "m", "created_at": "2026-10-01T00:00:00Z",
        }))
        .unwrap();
        p.model_version = model.map(str::to_owned);
        p.prompt_version = Some(prompt.into());
        p.reasons = reasons.iter().map(|r| (*r).to_owned()).collect();
        p
    }

    #[test]
    fn plan_regenerates_stale_and_placeholder_proposals() {
        let out = Kind::Outflow;
        let mut inputs = Inputs {
            transactions: vec![
                tx(1, "Fresh shop", None, out),
                tx(2, "Old model shop", None, out),
                tx(3, "Placeholder shop", None, out),
                tx(4, "Current shop", None, out),
                tx(5, "Categorised shop", Some("Groceries"), out),
                tx(6, "Deferred shop", None, out),
                tx(7, "unknown", None, out),
                tx(8, "Own transfer", None, Kind::Transfer),
            ],
            ..Inputs::default()
        };
        let old = proposal("old", 2, Some("jev-latest"), "category-v4-a", &[]);
        let placeholder = proposal("ph", 3, None, DAILY_PLACEHOLDER, &["awaiting_model"]);
        let current = proposal("cur", 4, Some("qwen3:8b"), "v", &["x"]);
        let categorised = proposal("cat", 5, Some("qwen3:8b"), "v", &[]);
        let gone = proposal("gone", 99, Some("qwen3:8b"), "v", &[]);
        inputs.pending = vec![
            old.clone(),
            placeholder.clone(),
            current.clone(),
            categorised.clone(),
            gone,
        ];
        inputs.latest = vec![old, placeholder, current, categorised];
        inputs.deferred.insert(inputs.transactions[5].source_key());
        inputs.daily_open.insert("firefly:3".into());
        let p = plan(
            &inputs,
            "qwen3:8b",
            "v",
            20,
            &HashMap::new(),
            Instant::now(),
        );
        assert_eq!(p.supersede, ["cat", "gone"]);
        // Daily first, then newest first.
        assert_eq!(p.candidates, [3, 2, 1]);
        assert_eq!(p.retry[&2], "old");
        assert_eq!(p.retry[&3], "ph");
        assert_eq!(
            plan(&inputs, "qwen3:8b", "v", 2, &HashMap::new(), Instant::now()).candidates,
            [3, 2]
        );
    }

    #[test]
    fn unavailable_proposals_wait_before_retrying() {
        let inputs = Inputs {
            transactions: vec![tx(1, "Shop", None, Kind::Outflow)],
            pending: vec![proposal(
                "p",
                1,
                Some("qwen3:8b"),
                "v",
                &["model_unavailable"],
            )],
            latest: vec![proposal(
                "p",
                1,
                Some("qwen3:8b"),
                "v",
                &["model_unavailable"],
            )],
            ..Inputs::default()
        };
        let now = Instant::now();
        assert_eq!(
            plan(&inputs, "qwen3:8b", "v", 20, &HashMap::new(), now).candidates,
            [1]
        );
        let waiting = HashMap::from([("p".to_owned(), now + RETRY_AFTER)]);
        assert!(
            plan(&inputs, "qwen3:8b", "v", 20, &waiting, now)
                .candidates
                .is_empty()
        );
    }

    /// The fixture database as a runner store.
    struct Fixture(RefCell<Db>, Cell<usize>);

    impl Store for Fixture {
        fn inputs(&self) -> std::result::Result<Inputs, String> {
            let db = self.0.borrow();
            let snap = db.snap();
            let conn = db.store.connection();
            Ok(Inputs {
                transactions: snap.transactions.clone(),
                pending: list_pending(conn).map_err(|e| e.to_string())?,
                latest: latest_proposals(conn).map_err(|e| e.to_string())?,
                daily_open: HashSet::new(),
                deferred: snap.deferred_sources.clone(),
            })
        }
        fn write(
            &self,
            f: &mut dyn FnMut(&Writer<'_>) -> Result<()>,
        ) -> std::result::Result<(), String> {
            self.0
                .borrow_mut()
                .write(|w, _| f(w))
                .map_err(|e| e.to_string())
        }
        fn done(&self) {
            self.1.set(self.1.get() + 1);
        }
    }

    /// Captured qwen3:8b answers: the classifier, then the second reviewer.
    struct Qwen3(RefCell<Vec<String>>);
    impl Generate for Qwen3 {
        fn generate(
            &self,
            prompt: &str,
            system: Option<&str>,
        ) -> std::result::Result<String, BackendError> {
            self.0.borrow_mut().push(prompt.to_owned());
            if system.is_some_and(|s| s.starts_with("You are a skeptical")) {
                return Ok(r#"{"verdict":"supported","reason":"Lidl is a supermarket.","evidence":["lidl"]}"#.into());
            }
            Ok(r#"{"category":"Groceries","confidence":0.94,"runner_up":"Dining","runner_up_confidence":0.03,
                "reason":"Lidl is a supermarket chain.","countries":{"from_country":null,"to_country":null},
                "extras":{"merchant_name":{"value":"Lidl","evidence":["lidl"]},"merchant_location":{"value":"Utrecht","evidence":["utrecht"]},"payment_method":null,"tags":[]}}"#.into())
        }
    }

    #[test]
    fn a_pass_stores_reviewable_proposals_and_replaces_old_ones() {
        let db = fixture::db();
        db.proposal(&"a".repeat(64), 10, Some("Groceries"));
        let store = Fixture(RefCell::new(db), Cell::new(0));
        let settings = AiSettings {
            classifier: Classifier::Ollama,
            ..AiSettings::default()
        };
        let model = Qwen3(RefCell::new(Vec::new()));
        let backends = Backends {
            settings: &settings,
            generate: Some(&model),
            embed: None,
            choose: None,
        };
        let mut runner = Runner::default();
        assert_eq!(runner.run(&store, backends).unwrap(), 2);
        assert_eq!(model.0.borrow().len(), 4);
        let db = store.0.borrow();
        let old: String = db.one(&format!(
            "SELECT status FROM category_proposals WHERE proposal_id = '{}'",
            "a".repeat(64)
        ));
        assert_eq!(old, "superseded");
        let latest = latest_proposals(db.store.connection()).unwrap();
        let p = latest
            .iter()
            .find(|p| p.source.external_id == "10")
            .unwrap();
        assert_eq!(p.status, ProposalStatus::Pending);
        assert_eq!(p.proposed_category.as_deref(), Some("Groceries"));
        assert_eq!(p.model_version.as_deref(), Some("qwen3:8b"));
        assert!(
            p.prompt_version
                .as_deref()
                .unwrap()
                .starts_with("category-v4-")
        );
        assert_eq!(p.sanity_check.status, CheckStatus::Supported);
        assert_eq!(
            p.extras.merchant_location.as_ref().unwrap().value,
            "Utrecht"
        );
        drop(db);
        // Nothing left to do: the proposal is current.
        assert_eq!(runner.run(&store, backends).unwrap(), 0);
        assert_eq!(model.0.borrow().len(), 4);
    }

    /// System One answers in the captured response shape: the category
    /// question, then the check.
    struct Jev(RefCell<usize>);
    impl Choose for Jev {
        fn choose(
            &self,
            request: &crate::ai::ChoiceRequest,
        ) -> std::result::Result<crate::ai::ChoiceResult, BackendError> {
            *self.0.borrow_mut() += 1;
            let options: Vec<String> = request.criteria.iter().map(|(o, _)| o.clone()).collect();
            let check = options.iter().any(|o| o == "supported");
            let (choice, other, top) = if check {
                ("supported", "review", 0.97)
            } else {
                ("Groceries", "Dining", 0.93)
            };
            let mut probabilities = serde_json::Map::new();
            for o in &options {
                probabilities.insert(o.clone(), serde_json::json!(0.0));
            }
            probabilities.insert(choice.into(), serde_json::json!(top));
            probabilities.insert(other.into(), serde_json::json!(1.0 - top));
            let answer = serde_json::json!({"model": "jev-latest", "answers": {"transaction_category": {
                "type": "choice", "choice": choice, "confidence": top, "probabilities": probabilities}}});
            crate::ai::typesafe::parse_choice(&answer, &options)
        }
    }

    #[test]
    fn a_jev_pass_checks_and_auto_applies_confident_proposals() {
        let store = Fixture(RefCell::new(fixture::db()), Cell::new(0));
        let settings = AiSettings {
            classifier: Classifier::TypeSafe,
            auto_apply: true,
            ..AiSettings::default()
        };
        let jev = Jev(RefCell::new(0));
        let backends = Backends {
            settings: &settings,
            generate: None,
            embed: None,
            choose: Some(&jev),
        };
        assert_eq!(Runner::default().run(&store, backends).unwrap(), 2);
        // Two category questions and two checks.
        assert_eq!(*jev.0.borrow(), 4);
        let mut db = store.0.into_inner();
        let latest = latest_proposals(db.store.connection()).unwrap();
        let p = latest
            .iter()
            .find(|p| p.source.external_id == "10")
            .unwrap();
        assert_eq!(p.model_version.as_deref(), Some("jev-latest"));
        assert_eq!(p.category_options[0].category, "Groceries");
        assert_eq!(p.runner_up.as_deref(), Some("Dining"));
        assert_eq!(p.sanity_check.status, CheckStatus::Supported);
        let applied = db
            .write(|w, snap| crate::ai::daily::pass(w, snap, true))
            .unwrap();
        assert_eq!(applied, 2);
        let category: String = db.one(
            "SELECT c.name FROM transactions t JOIN categories c ON c.id = t.category_id WHERE t.id = 10",
        );
        assert_eq!(category, "Groceries");
    }
}
