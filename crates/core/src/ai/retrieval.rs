//! Similar past examples for each merchant: text matches always, and
//! embedding neighbours when an embedding model is configured
//! (`retrieve_category_examples_lexical`, `retrieve_category_examples`,
//! `merge_category_examples`).

use std::collections::{HashMap, HashSet, VecDeque};

use super::difflib::SequenceMatcher;
use super::text::parse_description;
use super::{BackendError, Embed};
use crate::text::casefold;

pub const EMBEDDING_BATCH_SIZE: usize = 32;
pub const MAX_CACHED_EMBEDDINGS: usize = 4096;
pub const MAX_RETRIEVED_PER_QUERY: usize = 12;
pub const MAX_RETRIEVED_PER_CATEGORY: usize = 3;

pub type Example = (String, String);

const QWEN3_EMBEDDING_PREFIX: &str = "Instruct: Retrieve bank transactions with the same merchant or purchase \
purpose to support personal expense categorization.\nQuery: ";

/// Bank boilerplate that says nothing about the merchant.
const NOISE: [&str; 8] = [
    "number", "account", "email", "sepa", "iban", "bic", "naam", "nr",
];

/// Keep at most three examples per category and twelve in all.
fn diverse<'a>(ranked: impl Iterator<Item = &'a Example>) -> Vec<Example> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for example in ranked {
        let n = counts.entry(example.1.as_str()).or_default();
        if *n >= MAX_RETRIEVED_PER_CATEGORY {
            continue;
        }
        *n += 1;
        out.push(example.clone());
        if out.len() == MAX_RETRIEVED_PER_QUERY {
            break;
        }
    }
    out
}

fn unique(queries: &[String]) -> Vec<&String> {
    let mut seen = HashSet::new();
    queries.iter().filter(|q| seen.insert(q.as_str())).collect()
}

/// `_lexical_relevance` on pre-parsed retrieval texts.
fn relevance(query: &str, query_text: &str, merchant: &str, merchant_text: &str) -> f64 {
    if query == merchant {
        return 1.0;
    }
    if query_text == merchant_text {
        return 0.99;
    }
    let tokens = |text: &str| -> HashSet<String> {
        text.split_whitespace()
            .filter(|t| !NOISE.contains(t))
            .map(str::to_owned)
            .collect()
    };
    let (q, m) = (tokens(query_text), tokens(merchant_text));
    let shared = q.intersection(&m).count();
    let union = q.union(&m).count();
    let token_score = if union == 0 {
        0.0
    } else {
        shared as f64 / union as f64
    };
    let matcher = SequenceMatcher::new(query_text, merchant_text);
    if shared == 0 && matcher.quick_ratio() < 0.85 {
        return 0.0;
    }
    let character = matcher.ratio();
    if shared == 0 && character < 0.85 {
        return 0.0;
    }
    (0.7 * token_score + 0.3 * character).max(0.8 * character)
}

/// Text matches scoring at least 0.35, best first.
pub fn lexical(queries: &[String], corpus: &[Example]) -> HashMap<String, Vec<Example>> {
    let corpus_texts: Vec<String> = corpus
        .iter()
        .map(|(m, _)| parse_description(m).retrieval_text())
        .collect();
    let mut selected = HashMap::new();
    for query in unique(queries) {
        let query_text = parse_description(query).retrieval_text();
        let mut scored: Vec<(f64, &Example)> = corpus
            .iter()
            .zip(&corpus_texts)
            .map(|(e, text)| (relevance(query, &query_text, &e.0, text), e))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
        let ranked = scored
            .iter()
            .take_while(|(score, _)| *score >= 0.35)
            .map(|(_, e)| *e);
        selected.insert(query.clone(), diverse(ranked));
    }
    selected
}

/// `embedding_query`: Qwen3 embeddings take an instruction prefix.
pub fn embedding_query(query: &str, model: Option<&str>) -> String {
    let text = parse_description(query).retrieval_text();
    let qwen =
        model.is_some_and(|m| casefold(m.split(':').next().unwrap_or("")) == "qwen3-embedding");
    if qwen {
        let room = 512 - QWEN3_EMBEDDING_PREFIX.chars().count();
        format!(
            "{QWEN3_EMBEDDING_PREFIX}{}",
            text.chars().take(room).collect::<String>()
        )
    } else {
        text
    }
}

/// Embeddings of one model, reused across passes and bounded in size
/// (oldest first out).
#[derive(Debug, Default)]
pub struct EmbeddingCache {
    model: String,
    vectors: HashMap<String, Vec<f64>>,
    order: VecDeque<String>,
}

impl EmbeddingCache {
    fn for_model(&mut self, model: &str) {
        if self.model != model {
            *self = Self {
                model: model.to_owned(),
                ..Self::default()
            };
        }
    }

    fn insert(&mut self, text: String, vector: Vec<f64>) {
        if self.vectors.insert(text.clone(), vector).is_none() {
            self.order.push_back(text);
        }
    }

    fn trim(&mut self) {
        while self.order.len() > MAX_CACHED_EMBEDDINGS {
            if let Some(old) = self.order.pop_front() {
                self.vectors.remove(&old);
            }
        }
    }
}

fn cosine(left: &[f64], right: &[f64]) -> f64 {
    let dot: f64 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let l = left.iter().map(|v| v * v).sum::<f64>().sqrt();
    let r = right.iter().map(|v| v * v).sum::<f64>().sqrt();
    if l == 0.0 || r == 0.0 || left.len() != right.len() {
        -1.0
    } else {
        dot / (l * r)
    }
}

/// `retrieve_category_examples`: the nearest corpus merchants by cosine
/// similarity, category-diverse.
pub fn semantic(
    embedder: &dyn Embed,
    model: &str,
    queries: &[String],
    corpus: &[Example],
    cache: &mut EmbeddingCache,
) -> Result<HashMap<String, Vec<Example>>, BackendError> {
    if queries.is_empty() || corpus.is_empty() {
        return Ok(HashMap::new());
    }
    cache.for_model(model);
    let queries = unique(queries);
    let documents: Vec<String> = corpus
        .iter()
        .map(|(m, _)| parse_description(m).retrieval_text())
        .collect();
    let query_texts: Vec<String> = queries
        .iter()
        .map(|q| embedding_query(q, Some(model)))
        .collect();
    let texts: Vec<&String> = documents.iter().chain(&query_texts).collect();
    if texts
        .iter()
        .any(|t| t.is_empty() || t.chars().count() > 512)
    {
        return Err(BackendError::Invalid(
            "embedding inputs must be non-empty and bounded",
        ));
    }
    let mut seen = HashSet::new();
    let missing: Vec<String> = texts
        .iter()
        .filter(|t| !cache.vectors.contains_key(t.as_str()) && seen.insert(t.as_str()))
        .map(|t| (*t).clone())
        .collect();
    for batch in missing.chunks(EMBEDDING_BATCH_SIZE) {
        let vectors = embedder.embed(batch)?;
        if vectors.len() != batch.len() {
            return Err(BackendError::Protocol);
        }
        for (text, vector) in batch.iter().zip(vectors) {
            cache.insert(text.clone(), vector);
        }
    }
    let vectors: Vec<Vec<f64>> = texts
        .iter()
        .map(|t| cache.vectors.get(t.as_str()).cloned().unwrap_or_default())
        .collect();
    cache.trim();
    let (corpus_vectors, query_vectors) = vectors.split_at(corpus.len());
    let mut selected = HashMap::new();
    for (query, query_vector) in queries.into_iter().zip(query_vectors) {
        let mut ranked: Vec<(f64, &Example)> = corpus
            .iter()
            .zip(corpus_vectors)
            .map(|(e, v)| (cosine(query_vector, v), e))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.0.cmp(&b.1.0)));
        selected.insert(query.clone(), diverse(ranked.iter().map(|(_, e)| *e)));
    }
    Ok(selected)
}

pub const MAX_EMBEDDING_INPUTS: usize = 512;
pub const MAX_EMBEDDING_DIMENSIONS: usize = 4096;

/// Ollama `api/embed` inputs: 1–512 texts of 1–512 characters.
pub fn check_embedding_inputs(inputs: &[String]) -> Result<(), BackendError> {
    if !(1..=MAX_EMBEDDING_INPUTS).contains(&inputs.len())
        || inputs
            .iter()
            .any(|t| t.is_empty() || t.chars().count() > 512)
    {
        return Err(BackendError::Invalid(
            "embedding inputs must be non-empty and bounded",
        ));
    }
    Ok(())
}

/// The `embeddings` of an `api/embed` answer: one finite, non-empty vector
/// per input, all of one length (`OllamaClient.embed`).
pub fn parse_embeddings(
    document: &serde_json::Value,
    inputs: usize,
) -> Result<Vec<Vec<f64>>, BackendError> {
    let rows = document
        .get("embeddings")
        .and_then(serde_json::Value::as_array)
        .filter(|rows| rows.len() == inputs)
        .ok_or(BackendError::Protocol)?;
    let mut dimensions = None;
    rows.iter()
        .map(|row| {
            let row = row.as_array().ok_or(BackendError::Protocol)?;
            let vector: Vec<f64> = row
                .iter()
                .map(|v| {
                    v.as_f64()
                        .filter(|f| f.is_finite())
                        .ok_or(BackendError::Protocol)
                })
                .collect::<Result<_, _>>()?;
            if vector.is_empty() || vector.len() > MAX_EMBEDDING_DIMENSIONS {
                return Err(BackendError::Protocol);
            }
            if *dimensions.get_or_insert(vector.len()) != vector.len() {
                return Err(BackendError::Protocol);
            }
            Ok(vector)
        })
        .collect()
}

/// Text matches first, then semantic neighbours, still diverse and bounded.
pub fn merge(lexical: &[Example], semantic: &[Example]) -> Vec<Example> {
    let mut seen = HashSet::new();
    let unique: Vec<&Example> = lexical
        .iter()
        .chain(semantic)
        .filter(|e| seen.insert((*e).clone()))
        .collect();
    diverse(unique.into_iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn ex(m: &str, c: &str) -> Example {
        (m.to_owned(), c.to_owned())
    }

    #[test]
    fn lexical_keeps_close_text_matches_only() {
        let corpus = vec![
            ex("albert heijn number", "Groceries"),
            ex("albert heyn amsterdam", "Groceries"),
            ex("shell station", "Transport"),
        ];
        let found = lexical(&["albert heijn number".to_owned()], &corpus);
        let got = &found["albert heijn number"];
        assert_eq!(got[0], corpus[0]);
        assert!(got.iter().all(|e| e.1 == "Groceries"));
        assert!(!got.contains(&corpus[2]));
    }

    #[test]
    fn caps_per_category_and_merges_text_first() {
        let lexical: Vec<Example> = (0..5).map(|i| ex(&format!("a{i}"), "A")).collect();
        let semantic = vec![ex("b", "B"), ex("a0", "A")];
        let merged = merge(&lexical, &semantic);
        assert_eq!(
            merged,
            vec![ex("a0", "A"), ex("a1", "A"), ex("a2", "A"), ex("b", "B")]
        );
    }

    #[test]
    fn embeddings_are_validated() {
        let ok =
            serde_json::json!({"model": "embeddinggemma", "embeddings": [[0.1, -0.2], [1, 0]]});
        assert_eq!(parse_embeddings(&ok, 2).unwrap()[1], [1.0, 0.0]);
        for bad in [
            serde_json::json!({"embeddings": [[0.1, 0.2]]}),
            serde_json::json!({"embeddings": [[0.1, 0.2], [0.3]]}),
            serde_json::json!({"embeddings": [[0.1, true], [0.3, 0.1]]}),
            serde_json::json!({"embeddings": [[], []]}),
            serde_json::json!({}),
        ] {
            assert!(parse_embeddings(&bad, 2).is_err(), "{bad}");
        }
        assert!(check_embedding_inputs(&[String::new()]).is_err());
        assert!(check_embedding_inputs(&["a".repeat(513)]).is_err());
        assert!(check_embedding_inputs(&["ok".into()]).is_ok());
    }

    #[test]
    fn qwen3_embedding_queries_get_the_instruction() {
        let q = embedding_query("cafe noir", Some("qwen3-embedding:0.6b"));
        assert!(q.starts_with("Instruct: Retrieve bank transactions"));
        assert!(q.ends_with("\nQuery: cafe noir"));
        assert_eq!(
            embedding_query("cafe noir", Some("embeddinggemma")),
            "cafe noir"
        );
    }

    struct Fake {
        calls: Cell<usize>,
    }

    impl Embed for Fake {
        fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>, BackendError> {
            self.calls.set(self.calls.get() + 1);
            Ok(inputs
                .iter()
                .map(|t| {
                    if t.contains("coffee") || t.contains("cafe") {
                        vec![1.0, 0.1]
                    } else {
                        vec![0.0, 1.0]
                    }
                })
                .collect())
        }
    }

    #[test]
    fn semantic_ranks_by_cosine_and_caches() {
        let corpus = vec![
            ex("shell station", "Transport"),
            ex("coffee corner", "Dining"),
        ];
        let fake = Fake {
            calls: Cell::new(0),
        };
        let mut cache = EmbeddingCache::default();
        let found = semantic(
            &fake,
            "embeddinggemma",
            &["cafe noir".into()],
            &corpus,
            &mut cache,
        )
        .unwrap();
        assert_eq!(found["cafe noir"][0], corpus[1]);
        semantic(
            &fake,
            "embeddinggemma",
            &["cafe noir".into()],
            &corpus,
            &mut cache,
        )
        .unwrap();
        assert_eq!(fake.calls.get(), 1);
    }
}
