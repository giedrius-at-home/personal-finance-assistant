//! Category review inbox and daily review (Python dashboard._review_item,
//! _daily_inbox), memo country hints (domain/transaction_country.py) and the
//! Transactions tab search (transaction_search.py).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::sync::LazyLock;

use finance_types::gateway::{DailyReviewEntry, DailyReviewPolicy, SavedDetails};
use finance_types::report::Currency;
use finance_types::report::review::{
    CategoryProposal, CountryEvidence, DailyInbox, DailyInboxItem, MissedReviewMonth,
    ProposalStatus, ReviewItem, SearchAccount, SearchDirection, SearchRow, SearchTotal,
    TransactionSearchQuery, TransactionSearchResult,
};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use regex::Regex;
use rusqlite::Connection;
use rust_decimal::Decimal;

use crate::load::parse_instant;
use crate::model::{Account, Kind, Tx, source_key};
use crate::text::{self, casefold, is_country};
use crate::{Ctx, Error, Result};

/// The latest non-superseded proposal of every source, oldest row first,
/// with the lifecycle status from the table column.
pub fn latest_proposals(conn: &Connection) -> Result<Vec<CategoryProposal>> {
    let mut stmt = conn.prepare(
        "SELECT p.status, p.payload_json FROM category_proposals AS p \
         JOIN (SELECT MAX(rowid) AS latest_row FROM category_proposals \
         WHERE status != 'superseded' \
         GROUP BY source_system, source_entity, external_id) AS latest \
         ON p.rowid = latest.latest_row ORDER BY p.rowid",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(status, payload)| {
            let mut proposal: CategoryProposal = serde_json::from_str(&payload)
                .map_err(|e| Error::Invalid(format!("category proposal: {e}")))?;
            proposal.status = serde_json::from_value(serde_json::Value::String(status))
                .map_err(|e| Error::Invalid(format!("proposal status: {e}")))?;
            Ok(proposal)
        })
        .collect()
}

/// Pending proposals the owner can still act on, newest first: one per
/// transaction, skipping transfers and transactions that already have a category.
///
/// Python also dropped a proposal whose stored source revision differed from
/// the transaction's; the ledger keeps no source revisions, so only the
/// identity is compared here.
pub fn review_items(ctx: &Ctx<'_>) -> Vec<ReviewItem> {
    let banks = bank_names(ctx);
    reviewable(&ctx.snap.category_proposals, &ctx.snap.transactions)
        .into_iter()
        .map(|(p, t)| review_item(ctx, p, Some(t), &banks))
        .collect()
}

fn reviewable<'a>(
    proposals: &'a [CategoryProposal],
    transactions: &'a [Tx],
) -> Vec<(&'a CategoryProposal, &'a Tx)> {
    let by_ledger: HashMap<i64, &Tx> = transactions.iter().map(|t| (t.ledger_id, t)).collect();
    let mut found: Vec<(&CategoryProposal, &Tx, Option<Timestamp>)> = Vec::new();
    for proposal in proposals {
        if proposal.status != ProposalStatus::Pending {
            continue;
        }
        let Some(t) = proposal_ledger_id(proposal).and_then(|id| by_ledger.get(&id)) else {
            continue;
        };
        if t.kind == Kind::Transfer || t.category.is_some() {
            continue;
        }
        found.push((proposal, t, parse_instant(&proposal.created_at)));
    }
    // Stable, so equal instants keep table order as Python's sort did.
    found.sort_by_key(|a| std::cmp::Reverse(a.2));
    found.into_iter().map(|(p, t, _)| (p, t)).collect()
}

/// The daily review: every tracked transaction with its status and latest
/// proposal, plus missed reviews per month. `None` until daily review is set up.
///
/// `automation_active` says whether the daily worker is running; it is a
/// runtime fact, not stored state.
pub fn daily_inbox(ctx: &Ctx<'_>, automation_active: bool) -> Option<DailyInbox> {
    let snap = ctx.snap;
    let policy = snap.daily_review_policy.clone()?;
    let proposals: HashMap<String, &CategoryProposal> = snap
        .category_proposals
        .iter()
        .map(|p| (proposal_key(p), p))
        .collect();
    let banks = bank_names(ctx);
    let mut inbox = inbox(
        policy,
        &snap.daily_review_entries,
        &snap.transactions,
        &ctx.now,
        |t| {
            proposals
                .get(&t.source_key())
                .map(|p| review_item(ctx, p, Some(t), &banks))
        },
    );
    inbox.automation_active = automation_active;
    inbox.suggestions_active = automation_active;
    Some(inbox)
}

fn inbox(
    policy: DailyReviewPolicy,
    entries: &[DailyReviewEntry],
    transactions: &[Tx],
    now: &Zoned,
    review: impl Fn(&Tx) -> Option<ReviewItem>,
) -> DailyInbox {
    let zone = TimeZone::get(&policy.timezone).unwrap_or_else(|_| now.time_zone().clone());
    let by_id: HashMap<&str, &Tx> = transactions
        .iter()
        .map(|t| (t.transaction_id.as_str(), t))
        .collect();
    let instant = now.timestamp();
    let mut entries: Vec<_> = entries.iter().collect();
    entries.sort_by_key(|e| parse_instant(&e.first_seen));
    let mut missed: BTreeMap<String, u32> = BTreeMap::new();
    let mut still_open: BTreeMap<String, u32> = BTreeMap::new();
    let mut items = Vec::new();
    for entry in entries {
        let Some(t) = by_id.get(entry.transaction_id.as_str()) else {
            continue;
        };
        if t.kind == Kind::Transfer {
            continue;
        }
        let due = parse_instant(&entry.due_at);
        let is_missed = entry.missed_at.is_some()
            || (entry.resolved_at.is_none() && due.is_some_and(|d| instant >= d));
        if is_missed && let Some(due) = due {
            let month = month_key(due.to_zoned(zone.clone()).date());
            *missed.entry(month.clone()).or_default() += 1;
            if entry.resolved_at.is_none() {
                *still_open.entry(month).or_default() += 1;
            }
        }
        let status = if entry.resolved_at.is_some() {
            "done"
        } else if is_missed {
            "missed"
        } else {
            "pending"
        };
        items.push(DailyInboxItem {
            entry: entry.clone(),
            description: t.description.clone(),
            amount: Some(signed_amount(t)),
            currency: Some(t.currency.clone()),
            current_category: t.category.clone(),
            status: status.into(),
            proposal: review(t),
        });
    }
    let today = now.with_time_zone(zone.clone()).date();
    let mut months = Vec::new();
    if let Some(activated) = parse_instant(&policy.activated_at) {
        let mut cursor = activated.to_zoned(zone.clone()).date().first_of_month();
        let end = today.first_of_month();
        while cursor <= end {
            let key = month_key(cursor);
            months.push(MissedReviewMonth {
                missed: missed.get(&key).copied().unwrap_or(0),
                still_open: still_open.get(&key).copied().unwrap_or(0),
                month: key,
            });
            cursor = crate::period::shift_month(cursor, 1);
        }
    }
    DailyInbox {
        automation_active: false,
        suggestions_active: false,
        policy,
        today: today.to_string(),
        items,
        missed_months: months,
    }
}

fn review_item(
    ctx: &Ctx<'_>,
    proposal: &CategoryProposal,
    transaction: Option<&Tx>,
    banks: &HashMap<&str, &str>,
) -> ReviewItem {
    let snap = ctx.snap;
    let key = proposal_key(proposal);
    let p = proposal.clone();
    ReviewItem {
        deferred: snap.deferred_sources.contains(&key),
        bank_name: transaction
            .and_then(|t| banks.get(t.account_id.as_str()))
            .map(|name| (*name).to_owned()),
        extras: p.extras,
        owner_details: snap
            .details
            .get(&key)
            .cloned()
            .unwrap_or_else(SavedDetails::default),
        country_suggestions: p.country_suggestions,
        sanity_check: p.sanity_check,
        proposal_id: p.proposal_id,
        merchant: p.normalized_merchant,
        original_category: p.original_category,
        proposed_category: p.proposed_category,
        method: p.method,
        confidence: p.confidence,
        runner_up: p.runner_up,
        runner_up_confidence: p.runner_up_confidence,
        category_options: p.category_options,
        reasons: p.reasons,
        model_version: p.model_version,
        prompt_version: p.prompt_version,
        policy_version: p.policy_version,
        vocabulary_version: p.vocabulary_version,
        status: p.status,
        source_updated_at: p.source.source_updated_at.as_deref().map(python_instant),
        transaction_description: transaction.and_then(|t| t.description.clone()),
        transaction_amount: transaction.map(signed_amount),
        transaction_currency: transaction.map(|t| t.currency.clone()),
        transaction_occurred_at: transaction.map(|t| rfc3339(&t.occurred_at)),
        source_country: transaction.and_then(|t| t.source_country.as_ref().map(evidence)),
        destination_country: transaction.and_then(|t| t.destination_country.as_ref().map(evidence)),
        description_country: transaction
            .and_then(|t| description_country(t.description.as_deref()))
            .map(|e| evidence(&e)),
    }
}

fn proposal_ledger_id(proposal: &CategoryProposal) -> Option<i64> {
    let s = &proposal.source;
    (s.system == "firefly_iii" && s.entity == "transaction")
        .then(|| s.external_id.parse().ok())
        .flatten()
}

/// The `Tx::source_key` spelling of a proposal's source identity.
fn proposal_key(proposal: &CategoryProposal) -> String {
    match proposal_ledger_id(proposal) {
        Some(id) => source_key(id),
        None => {
            let s = &proposal.source;
            format!(
                "[\"{}\", \"{}\", \"{}\"]",
                s.system, s.entity, s.external_id
            )
        }
    }
}

fn bank_names<'a>(ctx: &Ctx<'a>) -> HashMap<&'a str, &'a str> {
    ctx.snap
        .accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.name.as_str()))
        .collect()
}

fn signed_amount(t: &Tx) -> Decimal {
    if t.kind == Kind::Outflow {
        -t.amount
    } else {
        t.amount
    }
}

fn month_key(day: Date) -> String {
    format!("{:04}-{:02}", day.year(), day.month())
}

/// RFC 3339 with the zone's offset; fractional seconds only when present.
fn rfc3339(at: &Zoned) -> String {
    at.timestamp().display_with_offset(at.offset()).to_string()
}

/// Pydantic writes a UTC instant with `Z`.
fn python_instant(text: &str) -> String {
    match text.strip_suffix("+00:00") {
        Some(head) => format!("{head}Z"),
        None => text.to_owned(),
    }
}

fn evidence(e: &text::CountryEvidence) -> CountryEvidence {
    CountryEvidence {
        code: e.code.clone(),
        basis: e.basis.clone(),
    }
}

/// A country stated outright in the memo (a country label, a labelled IBAN or
/// a BIC). Several different countries cancel each other out; merchant names
/// are never read as locations.
pub fn description_country(description: Option<&str>) -> Option<text::CountryEvidence> {
    static LABEL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:COUNTRY(?:_CODE)?|LAND(?:CODE)?)\s*[:=]?\s*([A-Z]{2,3})\b")
            .expect("valid regex")
    });
    static IBAN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\bIBAN\s*[:=]?\s*([A-Z]{2}[0-9]{2}[A-Z0-9]{11,30})\b").expect("valid regex")
    });
    static BIC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:BIC|SWIFT)\s*[:=]?\s*[A-Z0-9]{4}([A-Z]{2})[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b")
            .expect("valid regex")
    });
    let text = description.unwrap_or("").to_uppercase();
    // Insertion-ordered, like the Python dict.
    let mut candidates: Vec<text::CountryEvidence> = Vec::new();
    let mut add = |code: &str, basis: &str, replace: bool| match candidates
        .iter_mut()
        .find(|c| c.code == code)
    {
        Some(existing) if replace => existing.basis = basis.into(),
        Some(_) => {}
        None => candidates.push(text::CountryEvidence {
            code: code.into(),
            basis: basis.into(),
        }),
    };
    for c in LABEL.captures_iter(&text) {
        let raw = &c[1];
        let code = match raw {
            "UK" => "GB",
            _ => alpha3(raw).unwrap_or(raw),
        };
        if is_country(code) {
            add(code, "description_country", true);
        }
    }
    for c in IBAN.captures_iter(&text) {
        if let Some(found) = text::iban_country(Some(&c[1])) {
            add(&found.code, "description_iban", false);
        }
    }
    for c in BIC.captures_iter(&text) {
        if is_country(&c[1]) {
            add(&c[1], "description_bic", false);
        }
    }
    (candidates.len() == 1).then(|| candidates.remove(0))
}

/// Common statement alpha-3 labels; others stay unknown.
fn alpha3(code: &str) -> Option<&'static str> {
    const PAIRS: &str = "AUT:AT BEL:BE BGR:BG CHE:CH CYP:CY CZE:CZ DEU:DE DNK:DK ESP:ES EST:EE \
FIN:FI FRA:FR GBR:GB GRC:GR HRV:HR HUN:HU IRL:IE ISL:IS ITA:IT LTU:LT LUX:LU LVA:LV MLT:MT \
NLD:NL NOR:NO POL:PL PRT:PT ROU:RO SVK:SK SVN:SI SWE:SE TUR:TR UKR:UA USA:US CAN:CA AUS:AU \
NZL:NZ JPN:JP CHN:CN IND:IN BRA:BR ARE:AE SGP:SG MEX:MX ZAF:ZA";
    PAIRS
        .split(' ')
        .find_map(|pair| pair.strip_prefix(code)?.strip_prefix(':'))
}

const MAX_QUERY: usize = 128;
const MAX_PAGE_SIZE: u32 = 200;

/// Search the owner's transactions with the Transactions tab filters: all
/// query words must appear in the memo, counterparty, category or account
/// name. Newest first, one page, with per-currency totals of every match.
///
/// Fails with [`Error::Invalid`] on a filter the owner can fix.
pub fn search_transactions(
    ctx: &Ctx<'_>,
    query: &TransactionSearchQuery,
) -> Result<TransactionSearchResult> {
    search(&ctx.snap.transactions, &ctx.snap.accounts, query)
}

fn search(
    transactions: &[Tx],
    accounts: &[Account],
    query: &TransactionSearchQuery,
) -> Result<TransactionSearchResult> {
    let params = SearchParams::parse(query)?;
    let names: HashMap<&str, &str> = accounts
        .iter()
        .map(|a| (a.account_id.as_str(), a.name.as_str()))
        .collect();
    let tokens: Vec<String> = fold(&params.q)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut matched: Vec<(&Tx, Date, Option<String>)> = Vec::new();
    for t in transactions {
        let on = t.booked_on();
        let counterparty = if t.kind == Kind::Transfer {
            t.counterparty_account_id
                .as_deref()
                .and_then(|id| names.get(id))
                .map(|n| (*n).to_owned())
        } else {
            t.counterparty_name.clone()
        };
        if let Some(account) = &params.account {
            let incoming = (t.kind == Kind::Transfer)
                .then_some(t.counterparty_account_id.as_deref())
                .flatten();
            if *account != t.account_id && Some(account.as_str()) != incoming {
                continue;
            }
        }
        if params.start.is_some_and(|s| on < s) || params.end.is_some_and(|e| on > e) {
            continue;
        }
        if params.direction.is_some_and(|d| d != direction(t.kind)) {
            continue;
        }
        if params.min.is_some_and(|m| t.amount < m) || params.max.is_some_and(|m| t.amount > m) {
            continue;
        }
        if !tokens.is_empty() {
            let text = fold(
                &[
                    t.description.as_deref(),
                    counterparty.as_deref(),
                    t.category.as_deref(),
                    names.get(t.account_id.as_str()).copied(),
                ]
                .map(|part| part.unwrap_or(""))
                .join(" "),
            );
            if !tokens.iter().all(|token| text.contains(token.as_str())) {
                continue;
            }
        }
        matched.push((t, on, counterparty));
    }
    matched.sort_by(|a, b| a.0.transaction_id.cmp(&b.0.transaction_id));
    matched.sort_by(|a, b| {
        b.0.occurred_at
            .timestamp()
            .cmp(&a.0.occurred_at.timestamp())
    });

    let mut sums: BTreeMap<String, (Currency, Decimal, Decimal)> = BTreeMap::new();
    for (t, _, _) in &matched {
        let dir = direction(t.kind);
        if dir == SearchDirection::Transfer {
            continue;
        }
        let bucket = sums
            .entry(t.currency.code.clone())
            .or_insert_with(|| (t.currency.clone(), Decimal::ZERO, Decimal::ZERO));
        if dir == SearchDirection::In {
            bucket.1 += t.amount;
        } else {
            bucket.2 += t.amount;
        }
    }
    let totals = sums
        .into_values()
        .map(|(currency, inflow, outflow)| {
            let unit = u32::from(currency.minor_unit);
            SearchTotal {
                inflow: fixed(inflow, unit),
                outflow: fixed(outflow, unit),
                net: fixed(inflow - outflow, unit),
                currency,
            }
        })
        .collect();

    let total_count = matched.len();
    let first = (params.page as usize - 1).saturating_mul(params.page_size as usize);
    let items = matched
        .into_iter()
        .skip(first)
        .take(params.page_size as usize)
        .map(|(t, on, counterparty)| SearchRow {
            transaction_id: t.ledger_id.to_string(),
            occurred_on: on.to_string(),
            direction: direction(t.kind),
            amount: t.money(),
            counterparty,
            description: t.description.clone(),
            account_id: t.account_id.clone(),
            account_name: names.get(t.account_id.as_str()).map(|n| (*n).to_owned()),
            category: t.category.clone(),
            firefly_group_id: t.group_id.clone(),
        })
        .collect();
    let used: HashSet<&str> = transactions.iter().map(|t| t.account_id.as_str()).collect();
    let mut accounts: Vec<SearchAccount> = used
        .into_iter()
        .filter_map(|id| {
            names.get(id).map(|name| SearchAccount {
                account_id: id.to_owned(),
                name: (*name).to_owned(),
            })
        })
        .collect();
    accounts.sort_by(|a, b| {
        (casefold(&a.name), &a.account_id).cmp(&(casefold(&b.name), &b.account_id))
    });
    Ok(TransactionSearchResult {
        items,
        page: params.page,
        page_size: params.page_size,
        total_count,
        totals,
        accounts,
    })
}

fn direction(kind: Kind) -> SearchDirection {
    match kind {
        Kind::Inflow | Kind::Refund => SearchDirection::In,
        Kind::Outflow => SearchDirection::Out,
        Kind::Transfer => SearchDirection::Transfer,
    }
}

/// Rounded half-even to `unit` digits and shown with exactly that many.
fn fixed(value: Decimal, unit: u32) -> Decimal {
    let mut rounded = value.round_dp(unit);
    rounded.rescale(unit);
    rounded
}

/// Validated search filters.
struct SearchParams {
    q: String,
    account: Option<String>,
    start: Option<Date>,
    end: Option<Date>,
    direction: Option<SearchDirection>,
    min: Option<Decimal>,
    max: Option<Decimal>,
    page: u32,
    page_size: u32,
}

impl SearchParams {
    fn parse(query: &TransactionSearchQuery) -> Result<Self> {
        let invalid = |message: &str| Error::Invalid(message.to_owned());
        let q = query.q.trim().to_owned();
        if q.chars().count() > MAX_QUERY {
            return Err(invalid("search is too long"));
        }
        let day = |value: &Option<String>, key: &str| -> Result<Option<Date>> {
            match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                None => Ok(None),
                Some(raw) => Date::from_str(raw)
                    .map(Some)
                    .map_err(|_| invalid(&format!("{key} must be a date like 2026-10-10"))),
            }
        };
        let (start, end) = (day(&query.from, "from")?, day(&query.to, "to")?);
        if let (Some(s), Some(e)) = (start, end)
            && s > e
        {
            return Err(invalid("from must not be after to"));
        }
        for (value, key) in [(query.min, "min"), (query.max, "max")] {
            if value.is_some_and(|v| v.is_sign_negative() && !v.is_zero()) {
                return Err(invalid(&format!("{key} must be zero or more")));
            }
        }
        if let (Some(low), Some(high)) = (query.min, query.max)
            && low > high
        {
            return Err(invalid("min must not be above max"));
        }
        if !(1..=1_000_000).contains(&query.page) {
            return Err(invalid("page must be between 1 and 1000000"));
        }
        if !(1..=MAX_PAGE_SIZE).contains(&query.page_size) {
            return Err(invalid("page_size must be between 1 and 200"));
        }
        Ok(Self {
            q,
            account: query
                .account
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned),
            start,
            end,
            direction: query.direction,
            min: query.min,
            max: query.max,
            page: query.page,
            page_size: query.page_size,
        })
    }
}

/// Accent- and case-insensitive text (Python NFKD, drop combining marks,
/// casefold) for the Latin letters bank memos use.
fn fold(text: &str) -> String {
    static TABLE: LazyLock<HashMap<char, &'static str>> = LazyLock::new(|| {
        let mut table: HashMap<char, &'static str> = ACCENTED
            .chars()
            .zip(BASES.char_indices())
            .map(|(accented, (i, base))| (accented, &BASES[i..i + base.len_utf8()]))
            .collect();
        table.extend(MULTI.iter().copied());
        table
    });
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\u{0300}'..='\u{034E}' | '\u{0350}'..='\u{036F}') {
            continue;
        }
        match TABLE.get(&c) {
            Some(base) => out.push_str(base),
            None => out.push(c),
        }
    }
    casefold(&out)
}

/// Letters with a canonical or compatibility decomposition in Latin-1,
/// Latin Extended-A/B and Latin Extended Additional, and their bases
/// (generated with Python's `unicodedata`).
const ACCENTED: &str = "ÀÁÂÃÄÅÇÈÉÊËÌÍÎÏÑÒÓÔÕÖÙÚÛÜÝàáâãäåçèéêëìíîïñòóôõöùúûüýÿĀāĂăĄąĆćĈĉĊċČčĎďĒēĔĕĖėĘęĚěĜĝĞğĠġĢģĤĥĨĩĪīĬĭĮįİĴĵĶķĹĺĻļĽľŃńŅņŇňŌōŎŏŐőŔŕŖŗŘřŚśŜŝŞşŠšŢţŤťŨũŪūŬŭŮůŰűŲųŴŵŶŷŸŹźŻżŽžſƠơƯưǍǎǏǐǑǒǓǔǕǖǗǘǙǚǛǜǞǟǠǡǢǣǦǧǨǩǪǫǬǭǮǯǰǴǵǸǹǺǻǼǽǾǿȀȁȂȃȄȅȆȇȈȉȊȋȌȍȎȏȐȑȒȓȔȕȖȗȘșȚțȞȟȦȧȨȩȪȫȬȭȮȯȰȱȲȳḀḁḂḃḄḅḆḇḈḉḊḋḌḍḎḏḐḑḒḓḔḕḖḗḘḙḚḛḜḝḞḟḠḡḢḣḤḥḦḧḨḩḪḫḬḭḮḯḰḱḲḳḴḵḶḷḸḹḺḻḼḽḾḿṀṁṂṃṄṅṆṇṈṉṊṋṌṍṎṏṐṑṒṓṔṕṖṗṘṙṚṛṜṝṞṟṠṡṢṣṤṥṦṧṨṩṪṫṬṭṮṯṰṱṲṳṴṵṶṷṸṹṺṻṼṽṾṿẀẁẂẃẄẅẆẇẈẉẊẋẌẍẎẏẐẑẒẓẔẕẖẗẘẙẛẠạẢảẤấẦầẨẩẪẫẬậẮắẰằẲẳẴẵẶặẸẹẺẻẼẽẾếỀềỂểỄễỆệỈỉỊịỌọỎỏỐốỒồỔổỖỗỘộỚớỜờỞởỠỡỢợỤụỦủỨứỪừỬửỮữỰựỲỳỴỵỶỷỸỹ";
const BASES: &str = "AAAAAACEEEEIIIINOOOOOUUUUYaaaaaaceeeeiiiinooooouuuuyyAaAaAaCcCcCcCcDdEeEeEeEeEeGgGgGgGgHhIiIiIiIiIJjKkLlLlLlNnNnNnOoOoOoRrRrRrSsSsSsSsTtTtUuUuUuUuUuUuWwYyYZzZzZzsOoUuAaIiOoUuUuUuUuUuAaAaÆæGgKkOoOoƷʒjGgNnAaÆæØøAaAaEeEeIiIiOoOoRrRrUuUuSsTtHhAaEeOoOoOoOoYyAaBbBbBbCcDdDdDdDdDdEeEeEeEeEeFfGgHhHhHhHhHhIiIiKkKkKkLlLlLlLlMmMmMmNnNnNnNnOoOoOoOoPpPpRrRrRrRrSsSsSsSsSsTtTtTtTtUuUuUuUuUuVvVvWwWwWwWwWwXxXxYyZzZzZzhtwysAaAaAaAaAaAaAaAaAaAaAaAaEeEeEeEeEeEeEeEeIiIiOoOoOoOoOoOoOoOoOoOoOoOoUuUuUuUuUuUuUuYyYyYyYy";
const MULTI: [(char, &str); 18] = [
    ('Ĳ', "IJ"),
    ('ĳ', "ij"),
    ('Ŀ', "L·"),
    ('ŀ', "l·"),
    ('ŉ', "ʼn"),
    ('Ǆ', "DZ"),
    ('ǅ', "Dz"),
    ('ǆ', "dz"),
    ('Ǉ', "LJ"),
    ('ǈ', "Lj"),
    ('ǉ', "lj"),
    ('Ǌ', "NJ"),
    ('ǋ', "Nj"),
    ('ǌ', "nj"),
    ('Ǳ', "DZ"),
    ('ǲ', "Dz"),
    ('ǳ', "dz"),
    ('ẚ', "aʾ"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use finance_types::report::UnitMoney;
    use finance_types::report::review::{CategorisationMethod, ProposalSource};
    use serde_json::json;

    fn at(instant: &str) -> Zoned {
        instant
            .parse::<Timestamp>()
            .expect("valid instant")
            .to_zoned(TimeZone::get("Europe/Vilnius").expect("zone"))
    }

    fn tx(id: i64, kind: Kind, amount: i64, when: &str) -> Tx {
        Tx {
            transaction_id: format!("firefly:{id}"),
            ledger_id: id,
            account_id: "swedbank".into(),
            counterparty_account_id: None,
            kind,
            amount: Decimal::from(amount),
            currency: Currency::eur(),
            occurred_at: at(when),
            purchased_on: None,
            description: None,
            counterparty_name: None,
            group_id: None,
            category: None,
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
        }
    }

    fn account(id: &str, name: &str) -> Account {
        Account {
            account_id: id.into(),
            ledger_id: 0,
            name: name.into(),
            balance: UnitMoney {
                amount: Decimal::ZERO,
                currency: Currency::eur(),
            },
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    fn fixture() -> (Vec<Tx>, Vec<Account>) {
        let mut mom = tx(4349, Kind::Inflow, 500, "2026-10-10T12:00:00Z");
        mom.description = Some("Pinigų pervedimas".into());
        mom.category = Some("Money Transfer".into());
        mom.counterparty_name = Some("DAIVA MIRKLIENĖ".into());
        mom.group_id = Some("g-4349".into());
        let mut lidl = tx(1, Kind::Outflow, 20, "2026-10-01T09:00:00Z");
        lidl.description = Some("Card payment".into());
        lidl.counterparty_name = Some("Lidl".into());
        lidl.account_id = "revolut".into();
        let mut moved = tx(2, Kind::Transfer, 100, "2026-10-05T09:00:00Z");
        moved.counterparty_account_id = Some("revolut".into());
        let bare = tx(3, Kind::Outflow, 5, "2026-09-01T09:00:00Z");
        let mut dollars = tx(4, Kind::Inflow, 7, "2026-08-01T09:00:00Z");
        dollars.currency = Currency::new("USD");
        (
            vec![bare, moved, lidl, mom, dollars],
            vec![
                account("swedbank", "Swedbank"),
                account("revolut", "Revolut"),
            ],
        )
    }

    fn run(query: serde_json::Value) -> TransactionSearchResult {
        let (transactions, accounts) = fixture();
        let query = serde_json::from_value(query).expect("valid query");
        search(&transactions, &accounts, &query).expect("valid search")
    }

    fn ids(result: &TransactionSearchResult) -> Vec<&str> {
        result
            .items
            .iter()
            .map(|i| i.transaction_id.as_str())
            .collect()
    }

    #[test]
    fn accent_insensitive_multi_word_search_finds_counterparty() {
        let result = run(json!({"q": "daiva mirkliene"}));
        assert_eq!(ids(&result), ["4349"]);
        let item = &result.items[0];
        assert_eq!(item.counterparty.as_deref(), Some("DAIVA MIRKLIENĖ"));
        assert_eq!(item.account_name.as_deref(), Some("Swedbank"));
        assert_eq!(item.direction, SearchDirection::In);
        assert_eq!(item.firefly_group_id.as_deref(), Some("g-4349"));
        assert_eq!(item.amount.amount, Decimal::from(500));
    }

    #[test]
    fn whitespace_query_matches_everything_newest_first() {
        assert_eq!(ids(&run(json!({"q": "   "}))), ["4349", "2", "1", "3", "4"]);
        // Rows without text fields are still found by their account name.
        assert!(ids(&run(json!({"q": "swedbank"}))).contains(&"3"));
    }

    #[test]
    fn transfer_shows_counterparty_account_name() {
        let result = run(json!({"direction": "transfer"}));
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].counterparty.as_deref(), Some("Revolut"));
    }

    #[test]
    fn filters_combine() {
        assert_eq!(
            ids(&run(json!({"account": "revolut", "direction": "out"}))),
            ["1"]
        );
        assert_eq!(ids(&run(json!({"direction": "out"}))), ["1", "3"]);
        assert_eq!(ids(&run(json!({"min": "20", "max": "100"}))), ["2", "1"]);
        assert_eq!(
            ids(&run(json!({"from": "2026-10-01", "to": "2026-10-05"}))),
            ["2", "1"]
        );
        // Incoming transfers count for the receiving account.
        assert_eq!(ids(&run(json!({"account": "revolut"}))), ["2", "1"]);
    }

    #[test]
    fn date_filter_uses_owner_local_date() {
        let late = tx(5, Kind::Inflow, 1, "2026-10-09T23:30:00Z");
        let query = serde_json::from_value(json!({"from": "2026-10-10"})).expect("valid query");
        let result = search(&[late], &[], &query).expect("valid search");
        assert_eq!(ids(&result), ["5"]);
        assert_eq!(result.items[0].occurred_on, "2026-10-10");
    }

    #[test]
    fn paging_and_past_the_end() {
        let first = run(json!({"page": 1, "page_size": 2}));
        assert_eq!(ids(&first), ["4349", "2"]);
        assert_eq!(first.total_count, 5);
        assert_eq!(ids(&run(json!({"page": 3, "page_size": 2}))), ["4"]);
        let beyond = run(json!({"page": 9, "page_size": 2}));
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.total_count, 5);
    }

    #[test]
    fn totals_per_currency_exclude_transfers() {
        let result = run(json!({}));
        let eur = &result.totals[0];
        assert_eq!(eur.currency.code, "EUR");
        assert_eq!(
            [eur.inflow, eur.outflow, eur.net].map(|d| d.to_string()),
            ["500.00", "25.00", "475.00"]
        );
        assert_eq!(result.totals[1].inflow.to_string(), "7.00");
        let names: Vec<_> = result.accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Revolut", "Swedbank"]);
    }

    #[test]
    fn invalid_params_are_rejected() {
        for query in [
            json!({"q": "x".repeat(129)}),
            json!({"from": "2026-10-05", "to": "2026-10-01"}),
            json!({"from": "not-a-date"}),
            json!({"min": "-1"}),
            json!({"min": "5", "max": "1"}),
            json!({"page": 0}),
            json!({"page_size": 201}),
        ] {
            let query: TransactionSearchQuery =
                serde_json::from_value(query).expect("valid query shape");
            assert!(search(&[], &[], &query).is_err(), "{query:?}");
        }
        let fine = TransactionSearchQuery {
            q: " mom ".into(),
            from: Some("2026-10-01".into()),
            to: Some(String::new()),
            page_size: 200,
            ..TransactionSearchQuery::default()
        };
        assert!(search(&[], &[], &fine).is_ok());
    }

    #[test]
    fn description_hints_are_explicit_and_conflict_safe() {
        for (text, expected) in [
            ("Land: LTU", Some(("LT", "description_country"))),
            ("country=NL", Some(("NL", "description_country"))),
            ("Land DEU", Some(("DE", "description_country"))),
            ("IBAN: NL91ABNA0417164300", Some(("NL", "description_iban"))),
            ("BIC: DEUTDEFF", Some(("DE", "description_bic"))),
            ("Country ZZ", None),
            ("IN STORE NL OR US merchant", None),
            ("Country NL; IBAN DE89370400440532013000", None),
            ("(no description)", None),
        ] {
            let found = description_country(Some(text));
            assert_eq!(
                found.as_ref().map(|e| (e.code.as_str(), e.basis.as_str())),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn fold_strips_accents_and_case() {
        assert_eq!(ACCENTED.chars().count(), BASES.chars().count());
        assert_eq!(
            fold("DAIVA MIRKLIENĖ Straße Ĳssel"),
            "daiva mirkliene strasse ijssel"
        );
    }

    fn proposal(id: i64, created_at: &str, status: ProposalStatus) -> CategoryProposal {
        CategoryProposal {
            proposal_id: format!("{id:064x}"),
            source: ProposalSource {
                system: "firefly_iii".into(),
                entity: "transaction".into(),
                external_id: id.to_string(),
                source_revision: None,
                source_updated_at: None,
            },
            original_category: None,
            proposed_category: Some("Groceries".into()),
            method: CategorisationMethod::Model,
            confidence: None,
            runner_up: None,
            runner_up_confidence: None,
            category_options: Vec::new(),
            reasons: Vec::new(),
            model_version: None,
            prompt_version: None,
            policy_version: "p".into(),
            vocabulary_version: "v".into(),
            feature_hash: "0".repeat(64),
            normalized_merchant: "merchant".into(),
            status,
            created_at: created_at.into(),
            sanity_check: Default::default(),
            extras: Default::default(),
            country_suggestions: Default::default(),
        }
    }

    #[test]
    fn review_skips_decided_transfers_and_categorised_newest_first() {
        let mut categorised = tx(3, Kind::Outflow, 1, "2026-10-01T09:00:00Z");
        categorised.category = Some("Rent".into());
        let transactions = vec![
            tx(1, Kind::Outflow, 1, "2026-10-01T09:00:00Z"),
            tx(2, Kind::Outflow, 1, "2026-10-01T09:00:00Z"),
            categorised,
            tx(4, Kind::Transfer, 1, "2026-10-01T09:00:00Z"),
            tx(6, Kind::Outflow, 1, "2026-10-01T09:00:00Z"),
        ];
        let proposals = vec![
            proposal(1, "2026-10-02T09:00:00Z", ProposalStatus::Pending),
            proposal(2, "2026-10-03T09:00:00.5Z", ProposalStatus::Pending),
            proposal(3, "2026-10-04T09:00:00Z", ProposalStatus::Pending),
            proposal(4, "2026-10-04T09:00:00Z", ProposalStatus::Pending),
            // No such transaction (deleted at the bank).
            proposal(5, "2026-10-04T09:00:00Z", ProposalStatus::Pending),
            proposal(6, "2026-10-04T09:00:00Z", ProposalStatus::Approved),
        ];
        let found: Vec<_> = reviewable(&proposals, &transactions)
            .into_iter()
            .map(|(_, t)| t.ledger_id)
            .collect();
        assert_eq!(found, [2, 1]);
    }

    fn entry(id: i64, due_at: &str) -> DailyReviewEntry {
        DailyReviewEntry {
            transaction_id: format!("firefly:{id}"),
            batch_date: "2026-09-15".into(),
            first_seen: format!("2026-09-15T00:00:0{id}Z"),
            due_at: due_at.into(),
            resolved_at: None,
            resolution: None,
            missed_at: None,
            auto_error: None,
            auto_proposal_id: None,
            auto_category: None,
            audited_at: None,
        }
    }

    #[test]
    fn missed_reviews_are_counted_per_month_and_transfers_hidden() {
        let policy = DailyReviewPolicy {
            activated_at: "2026-08-20T10:00:00Z".into(),
            timezone: "Europe/Amsterdam".into(),
            auto_threshold: Decimal::new(90, 2),
            missed_after_days: 30,
        };
        // Missed the deadline, completed late: still counted as missed.
        let mut late_done = entry(2, "2026-10-15T00:00:00Z");
        late_done.missed_at = Some("2026-10-15T00:00:00Z".into());
        late_done.resolved_at = Some("2026-10-16T00:00:00Z".into());
        let entries = vec![
            entry(1, "2026-10-15T00:00:00Z"),
            late_done,
            entry(3, "2026-11-15T00:00:00Z"),
            entry(4, "2026-10-01T00:00:00Z"),
            entry(9, "2026-10-01T00:00:00Z"),
        ];
        let transactions = vec![
            tx(1, Kind::Outflow, 3, "2026-09-14T09:00:00Z"),
            tx(2, Kind::Inflow, 4, "2026-09-14T09:00:00Z"),
            tx(3, Kind::Outflow, 5, "2026-09-14T09:00:00Z"),
            tx(4, Kind::Transfer, 6, "2026-09-14T09:00:00Z"),
        ];
        let now = at("2026-10-26T20:00:00Z");
        let inbox = inbox(policy, &entries, &transactions, &now, |_| None);
        let statuses: Vec<_> = inbox
            .items
            .iter()
            .map(|i| (i.entry.transaction_id.as_str(), i.status.as_str()))
            .collect();
        assert_eq!(
            statuses,
            [
                ("firefly:1", "missed"),
                ("firefly:2", "done"),
                ("firefly:3", "pending")
            ]
        );
        assert_eq!(inbox.items[0].amount, Some(Decimal::from(-3)));
        assert_eq!(inbox.today, "2026-10-26");
        let months: Vec<_> = inbox
            .missed_months
            .iter()
            .map(|m| (m.month.as_str(), m.missed, m.still_open))
            .collect();
        assert_eq!(
            months,
            [("2026-08", 0, 0), ("2026-09", 0, 0), ("2026-10", 2, 1)]
        );
    }
}
