//! The service side of the AI features: settings, inputs for group
//! suggestions, the research cache and the review checks the commands make.

use std::collections::HashSet;
use std::str::FromStr;

use finance_types::ai::{AiSettings, ResearchResult};
use finance_types::gateway::DailyReviewPolicy;
use jiff::Timestamp;
use rust_decimal::Decimal;

use super::groups::GroupInput;
use super::research::{self, Cache};
use super::settings;
use crate::groups::expense_categories;
use crate::service::Service;
use crate::writes::invalid;

impl Service {
    /// The owner's AI settings (defaults until saved).
    pub fn ai_settings(&self) -> Result<AiSettings, String> {
        match self.read_store(settings::load) {
            Err(e) if e == crate::service::NO_DATA => Ok(AiSettings::default()),
            other => other,
        }
    }

    /// Check and store the settings. Turning automatic approval on starts
    /// the daily review if it never ran: transactions so far are history.
    pub fn save_ai_settings(&self, new: AiSettings) -> Result<AiSettings, String> {
        self.write(|w, snap| {
            let saved = settings::save(w, new)?;
            if saved.auto_apply && snap.daily_review_policy.is_none() {
                let policy = DailyReviewPolicy {
                    activated_at: w.stamp(),
                    timezone: snap.settings.timezone.clone(),
                    auto_threshold: Decimal::from_str("0.90").unwrap_or_default(),
                    missed_after_days: 30,
                };
                w.put_json(
                    "INSERT OR IGNORE INTO daily_review_policy VALUES (1, ?1)",
                    &policy,
                    &[],
                )?;
                for t in &snap.raw_transactions {
                    w.tx.execute(
                        "INSERT OR IGNORE INTO daily_review_baseline VALUES (?1)",
                        [&t.transaction_id],
                    )?;
                }
            }
            Ok(saved)
        })
    }

    /// What a group suggestion for `category` is based on; the category must
    /// be one that spending uses (`/spending-groups/suggest`).
    pub fn group_input(&self, category: &str) -> Result<GroupInput, String> {
        let category = category.trim();
        if category.is_empty() || category.chars().count() > 128 {
            return Err("Invalid category.".into());
        }
        self.read(|_, snap| {
            if !expense_categories(&snap.transactions).contains(category) {
                return Err(invalid("This category is not used by spending."));
            }
            let mut seen = HashSet::new();
            let descriptions = snap
                .transactions
                .iter()
                .filter(|t| t.category.as_deref() == Some(category))
                .filter_map(|t| t.description.as_deref())
                .filter(|d| !d.is_empty())
                .map(|d| d.chars().take(256).collect::<String>())
                .filter(|d| seen.insert(d.clone()))
                .collect();
            Ok(GroupInput {
                category: category.to_owned(),
                mappings: snap.group_mappings.clone(),
                custom_groups: snap.custom_groups.clone(),
                descriptions,
                public_research: None,
            })
        })
    }

    /// The proposal is still pending (research and manual actions).
    pub fn proposal_pending(&self, proposal_id: &str) -> Result<bool, String> {
        if proposal_id.len() != 64 || !proposal_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(false);
        }
        self.read_store(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM category_proposals WHERE proposal_id = ?1 AND status = 'pending')",
                [proposal_id],
                |r| r.get(0),
            )?)
        })
    }
}

/// The research cache lives in the app database. A missing table (before
/// the first write) or a failed write only costs a refetch.
impl Cache for Service {
    fn get(&self, key: &str) -> Option<(ResearchResult, Timestamp)> {
        self.read_store(|conn| research::cached(conn, key))
            .ok()
            .flatten()
    }

    fn put(&self, result: &ResearchResult) {
        let _ = self.write_raw(|w| research::store(&w.tx, result));
    }
}
