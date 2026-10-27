//! Public research with citations (Python `domain/research.py`,
//! `research.py`, `research_cache.py`): only internally built HTTPS URLs on
//! exact public hosts, only sanitised public names, results cached in the
//! app database. Off unless the owner turns it on.

use std::str::FromStr;
use std::sync::LazyLock;

use finance_types::ai::{
    MarketIdentifierKind, MarketQuestion, MarketResearchResult, MerchantResearchResult,
    PublicCitation, ResearchPurpose, ResearchResult, ResearchStatus, UntrustedPublicDocument,
};
use jiff::{SignedDuration, Timestamp};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::extract::fold;
use super::pyjson::J;
use crate::writes::transactions::sha256_hex;
use crate::writes::{invalid, utc_stamp};
use crate::{Error, Result};

pub const POLICY_VERSION: &str = "2026-09-v1";
pub const SCHEMA_VERSION: &str = "2026-09-v1";
pub const MAX_PUBLIC_NAME_LENGTH: usize = 80;
pub const MAX_RESEARCH_BYTES: usize = 256 * 1024;
pub const MAX_REDIRECTS: usize = 2;
pub const USER_AGENT: &str = "personal-finance-assistant/0.1 (https://github.com/giedrius-at-home/personal-finance-assistant)";
/// How long a cited document stays fresh.
pub const TTL: SignedDuration = SignedDuration::from_hours(24);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    Wikipedia,
    SecEdgar,
}

impl Provider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wikipedia => "wikipedia",
            Self::SecEdgar => "sec_edgar",
        }
    }

    /// The only host each provider may be fetched from.
    pub fn host(self) -> &'static str {
        match self {
            Self::Wikipedia => "en.wikipedia.org",
            Self::SecEdgar => "www.sec.gov",
        }
    }
}

/// A fetched public document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    /// The final URL after redirects.
    pub url: String,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// GETs of internally built URLs. The implementation checks every hop with
/// [`allowed`], resolves the host to public addresses only, follows at most
/// [`MAX_REDIRECTS`] redirects and reads at most [`MAX_RESEARCH_BYTES`].
/// Errors are short codes (`network_unavailable`, `redirect_limit`, …).
pub trait Fetch {
    fn get(&self, url: &str, provider: Provider) -> std::result::Result<Fetched, String>;
}

/// `_validate_destination` / `_validate_citation_url` without DNS: HTTPS on
/// the provider's exact host and port 443, no credentials or fragment.
pub fn allowed(url: &str, provider: Provider) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().map(str::to_ascii_lowercase).as_deref() == Some(provider.host())
            && u.port().is_none_or(|p| p == 443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
    })
}

/// A sanitised public merchant name (`MerchantResearchRequest`).
pub fn public_name(value: &str) -> Result<String> {
    static EMAIL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[^\s@]+@[^\s@]+\.[^\s@]+").expect("valid regex"));
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(?:https?://|www\.)").expect("valid regex"));
    static IBAN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").expect("valid regex"));
    static LONG_NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\d{4,}").expect("valid regex"));
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\w .,&'()\-/]+$").expect("valid regex"));
    let rejected = || invalid("Enter only a public store or merchant name.");
    if !(2..=MAX_PUBLIC_NAME_LENGTH).contains(&value.chars().count()) {
        return Err(rejected());
    }
    let name = value.trim();
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{e000}'..='\u{f8ff}'))
        || EMAIL.is_match(name)
        || URL.is_match(name)
        || IBAN.is_match(name)
        || LONG_NUMBER.is_match(name)
        || !NAME.is_match(name)
    {
        return Err(rejected());
    }
    Ok(name.to_owned())
}

/// `ResearchQuery`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub provider: Provider,
    pub purpose: ResearchPurpose,
    pub public_subject: String,
    pub country_code: Option<String>,
    pub identifier_kind: Option<MarketIdentifierKind>,
}

impl Query {
    pub fn merchant(name: &str, country_code: Option<&str>) -> Self {
        let subject = match country_code {
            Some(cc) => format!("{name} {cc} company"),
            None => name.to_owned(),
        };
        Self {
            provider: Provider::Wikipedia,
            purpose: ResearchPurpose::MerchantIdentity,
            public_subject: subject,
            country_code: country_code.map(str::to_owned),
            identifier_kind: None,
        }
    }

    pub fn market(identifier: &str, kind: MarketIdentifierKind, question: MarketQuestion) -> Self {
        Self {
            provider: Provider::SecEdgar,
            purpose: match question {
                MarketQuestion::CompanyProfile => ResearchPurpose::MarketCompanyProfile,
                MarketQuestion::RegulatoryFilings => ResearchPurpose::MarketRegulatoryFilings,
                MarketQuestion::RiskDisclosures => ResearchPurpose::MarketRiskDisclosures,
            },
            public_subject: identifier.to_owned(),
            country_code: None,
            identifier_kind: Some(kind),
        }
    }

    /// `ResearchQuery.key`.
    pub fn key(&self) -> String {
        let kind = self.identifier_kind.map(|k| match k {
            MarketIdentifierKind::Ticker => "ticker",
            MarketIdentifierKind::Cik => "cik",
            MarketIdentifierKind::Isin => "isin",
        });
        sha256_hex(
            &J::obj([
                ("provider", J::str(self.provider.as_str())),
                ("purpose", J::str(self.purpose.as_str())),
                ("public_subject", J::str(&self.public_subject)),
                ("country_code", J::opt(self.country_code.clone())),
                ("identifier_kind", J::opt(kind)),
            ])
            .dumps_sorted_compact(),
        )
    }

    /// `_query_url`.
    pub fn url(&self) -> std::result::Result<String, String> {
        let encode = |pairs: &[(&str, &str)]| {
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(pairs)
                .finish()
        };
        match self.provider {
            Provider::Wikipedia => Ok(format!(
                "https://en.wikipedia.org/w/api.php?{}",
                encode(&[
                    ("action", "query"),
                    ("exintro", "1"),
                    ("explaintext", "1"),
                    ("format", "json"),
                    ("formatversion", "2"),
                    ("generator", "search"),
                    ("gsrlimit", "3"),
                    ("gsrsearch", &self.public_subject),
                    ("inprop", "url"),
                    ("maxlag", "5"),
                    ("prop", "extracts|info"),
                ])
            )),
            Provider::SecEdgar => {
                if self.identifier_kind == Some(MarketIdentifierKind::Isin) {
                    return Err("unsupported_identifier_for_provider".into());
                }
                Ok(format!(
                    "https://www.sec.gov/cgi-bin/browse-edgar?{}",
                    encode(&[
                        ("CIK", &self.public_subject),
                        ("action", "getcompany"),
                        ("count", "10"),
                        ("output", "atom"),
                    ])
                ))
            }
        }
    }
}

/// `MarketResearchRequest`: a ticker, CIK or ISIN, upper-cased.
pub fn market_identifier(identifier: &str, kind: MarketIdentifierKind) -> Result<String> {
    static TICKER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9.-]{0,11}$").expect("valid regex"));
    static CIK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,10}$").expect("valid regex"));
    static ISIN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Z]{2}[A-Z0-9]{9}\d$").expect("valid regex"));
    let upper = identifier.to_uppercase();
    let valid = (1..=12).contains(&identifier.chars().count())
        && match kind {
            MarketIdentifierKind::Ticker => TICKER.is_match(&upper),
            MarketIdentifierKind::Cik => CIK.is_match(&upper),
            MarketIdentifierKind::Isin => ISIN.is_match(&upper),
        };
    valid
        .then_some(upper)
        .ok_or_else(|| invalid("invalid public market identifier"))
}

fn document(
    title: &str,
    publisher: &str,
    url: &str,
    excerpt: &str,
    now: Timestamp,
) -> std::result::Result<UntrustedPublicDocument, String> {
    let squash = |s: &str, n: usize| -> String {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(n)
            .collect()
    };
    let title = squash(title, 300);
    let excerpt = squash(excerpt, 2000);
    if title.is_empty() || excerpt.is_empty() {
        return Err("empty_document".into());
    }
    let expires = now
        .checked_add(TTL)
        .map_err(|_| "empty_document".to_owned())?;
    Ok(UntrustedPublicDocument {
        trust_label: "untrusted_public_web".into(),
        citation: PublicCitation {
            title,
            publisher: publisher.into(),
            url: url.into(),
            retrieved_at: utc_stamp(now),
            expires_at: utc_stamp(expires),
            content_hash: sha256_hex(&excerpt),
        },
        excerpt,
    })
}

fn wikipedia(
    body: &[u8],
    now: Timestamp,
) -> std::result::Result<Vec<UntrustedPublicDocument>, String> {
    let payload: Value = serde_json::from_slice(body).map_err(|_| "invalid_json".to_owned())?;
    let payload = payload.as_object().ok_or("invalid_schema")?;
    let pages = match payload.get("query") {
        Some(Value::Object(q)) => q.get("pages").cloned().unwrap_or(Value::Array(Vec::new())),
        _ => Value::Array(Vec::new()),
    };
    let pages = pages.as_array().ok_or("invalid_schema")?;
    let mut documents = Vec::new();
    for page in pages.iter().take(3) {
        let page = page.as_object().ok_or("invalid_schema")?;
        let field = |k: &str| {
            page.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
        };
        let (Some(title), Some(extract), Some(url)) =
            (field("title"), field("extract"), field("fullurl"))
        else {
            continue;
        };
        if !allowed(url, Provider::Wikipedia) {
            return Err("citation_rejected".into());
        }
        documents.push(document(title, "Wikipedia", url, extract, now)?);
    }
    Ok(documents)
}

/// Text of an HTML page (`_html_text`): tags removed, entities decoded.
fn html_text(body: &[u8]) -> std::result::Result<String, String> {
    let text = std::str::from_utf8(body).map_err(|_| "unsupported_encoding".to_owned())?;
    let mut out = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    Ok(out
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(2000)
        .collect())
}

const SEC: &str = "U.S. Securities and Exchange Commission";

fn sec_atom(
    body: &[u8],
    fallback: &str,
    now: Timestamp,
) -> std::result::Result<Vec<UntrustedPublicDocument>, String> {
    const ATOM: &str = "http://www.w3.org/2005/Atom";
    let text = std::str::from_utf8(body).map_err(|_| "invalid_xml".to_owned())?;
    let xml = roxmltree::Document::parse(text).map_err(|_| "invalid_xml".to_owned())?;
    fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
        node.children().find(|c| {
            c.is_element() && c.tag_name().namespace() == Some(ATOM) && c.tag_name().name() == name
        })
    }
    let mut documents = Vec::new();
    let entries = xml.root_element().children().filter(|c| {
        c.is_element() && c.tag_name().namespace() == Some(ATOM) && c.tag_name().name() == "entry"
    });
    for entry in entries.take(3) {
        let text_of = |name: &str| {
            child(entry, name)
                .and_then(|n| n.text())
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let (title, summary) = (text_of("title"), text_of("summary"));
        let url = child(entry, "link").map_or(fallback.to_owned(), |l| {
            l.attribute("href").unwrap_or("").to_owned()
        });
        if title.is_empty() || summary.is_empty() {
            continue;
        }
        if !allowed(&url, Provider::SecEdgar) {
            return Err("citation_rejected".into());
        }
        documents.push(document(&title, SEC, &url, &summary, now)?);
    }
    Ok(documents)
}

/// `_parse_response`.
pub fn parse(
    provider: Provider,
    fetched: &Fetched,
    now: Timestamp,
) -> std::result::Result<Vec<UntrustedPublicDocument>, String> {
    let media = crate::text::casefold(fetched.content_type.split(';').next().unwrap_or("").trim());
    match provider {
        Provider::Wikipedia if media == "application/json" => wikipedia(&fetched.body, now),
        Provider::Wikipedia => Err("unsupported_content_type".into()),
        Provider::SecEdgar => match media.as_str() {
            "text/html" => {
                let text = html_text(&fetched.body)?;
                if text.is_empty() {
                    return Ok(Vec::new());
                }
                Ok(vec![document(
                    "SEC EDGAR company search",
                    SEC,
                    &fetched.url,
                    &text,
                    now,
                )?])
            }
            "application/atom+xml" | "application/xml" | "text/xml" => {
                sec_atom(&fetched.body, &fetched.url, now)
            }
            _ => Err("unsupported_content_type".into()),
        },
    }
}

fn empty(query: &Query, status: ResearchStatus, now: Timestamp, code: &str) -> ResearchResult {
    ResearchResult {
        schema_version: SCHEMA_VERSION.into(),
        policy_version: POLICY_VERSION.into(),
        query_key: query.key(),
        purpose: query.purpose,
        status,
        documents: Vec::new(),
        error_code: Some(code.into()),
        generated_at: utc_stamp(now),
    }
}

/// The research cache (`research_cache`).
pub trait Cache {
    /// A cached result and when it expires.
    fn get(&self, key: &str) -> Option<(ResearchResult, Timestamp)>;
    fn put(&self, result: &ResearchResult);
}

/// `RestrictedResearchGateway.retrieve`: fresh cache, else live, else stale
/// cache, else unavailable. Never errors.
pub fn retrieve(
    enabled: bool,
    cache: &dyn Cache,
    fetch: &dyn Fetch,
    query: &Query,
    now: Timestamp,
) -> ResearchResult {
    if !enabled {
        return empty(query, ResearchStatus::Disabled, now, "research_disabled");
    }
    let key = query.key();
    let cached = cache.get(&key);
    if let Some((result, expires)) = &cached
        && *expires > now
    {
        return ResearchResult {
            status: ResearchStatus::FreshCache,
            generated_at: utc_stamp(now),
            ..result.clone()
        };
    }
    let fetched = query
        .url()
        .and_then(|url| fetch.get(&url, query.provider))
        .and_then(|f| parse(query.provider, &f, now));
    match fetched {
        Err(code) => match cached {
            Some((result, _)) => ResearchResult {
                status: ResearchStatus::StaleCache,
                generated_at: utc_stamp(now),
                error_code: Some(code),
                ..result
            },
            None => empty(query, ResearchStatus::Unavailable, now, &code),
        },
        Ok(documents) if documents.is_empty() => {
            empty(query, ResearchStatus::Abstained, now, "no_cited_evidence")
        }
        Ok(documents) => {
            let result = ResearchResult {
                schema_version: SCHEMA_VERSION.into(),
                policy_version: POLICY_VERSION.into(),
                query_key: key,
                purpose: query.purpose,
                status: ResearchStatus::Live,
                documents,
                error_code: None,
                generated_at: utc_stamp(now),
            };
            cache.put(&result);
            result
        }
    }
}

/// `RestrictedResearchGateway.merchant`: keep the one page whose title is
/// the name itself; several or none means the identity is ambiguous.
pub fn merchant(
    enabled: bool,
    cache: &dyn Cache,
    fetch: &dyn Fetch,
    name: &str,
    country_code: Option<&str>,
    now: Timestamp,
) -> MerchantResearchResult {
    let mut result = retrieve(
        enabled,
        cache,
        fetch,
        &Query::merchant(name, country_code),
        now,
    );
    if !result.documents.is_empty() {
        let expected = fold(name);
        let exact: Vec<UntrustedPublicDocument> = result
            .documents
            .iter()
            .filter(|d| fold(&d.citation.title) == expected)
            .cloned()
            .collect();
        if exact.len() == 1 {
            result.documents = exact;
        } else {
            result.status = ResearchStatus::Abstained;
            result.documents = Vec::new();
            result.error_code = Some("ambiguous_public_identity".into());
        }
    }
    MerchantResearchResult {
        schema_version: SCHEMA_VERSION.into(),
        public_name: name.to_owned(),
        country_code: country_code.map(str::to_owned),
        research: result,
        likely_business_type: None,
        interpretation: "owner_or_local_model_review_required".into(),
        category_change_allowed: false,
    }
}

/// `RestrictedResearchGateway.market`.
pub fn market(
    enabled: bool,
    cache: &dyn Cache,
    fetch: &dyn Fetch,
    identifier: &str,
    kind: MarketIdentifierKind,
    question: MarketQuestion,
    now: Timestamp,
) -> Result<MarketResearchResult> {
    let identifier = market_identifier(identifier, kind)?;
    let research = retrieve(
        enabled,
        cache,
        fetch,
        &Query::market(&identifier, kind, question),
        now,
    );
    Ok(MarketResearchResult {
        schema_version: SCHEMA_VERSION.into(),
        identifier,
        identifier_kind: kind,
        question,
        research,
        personalized_advice: false,
        trade_action_allowed: false,
    })
}

// ---------------------------------------------------------------------------
// The cache in the app database

/// A cached result and its expiry.
pub fn cached(conn: &Connection, key: &str) -> Result<Option<(ResearchResult, Timestamp)>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT result_json, expires_at FROM research_cache WHERE query_key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((json, expires)) = row else {
        return Ok(None);
    };
    let result = serde_json::from_str(&json).map_err(|e| Error::Invalid(e.to_string()))?;
    let expires = Timestamp::from_str(&expires).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(Some((result, expires)))
}

/// Store a cited result (`ResearchCache.put`).
pub fn store(conn: &Connection, result: &ResearchResult) -> Result<()> {
    let earliest = |f: fn(&PublicCitation) -> &str| {
        result
            .documents
            .iter()
            .filter_map(|d| Timestamp::from_str(f(&d.citation)).ok())
            .min()
    };
    let (Some(expires), Some(retrieved)) = (
        earliest(|c| c.expires_at.as_str()),
        earliest(|c| c.retrieved_at.as_str()),
    ) else {
        return Err(invalid("only cited public research can be cached"));
    };
    let iso = |t: Timestamp| t.strftime("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string();
    let json = serde_json::to_string(result).map_err(|e| Error::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO research_cache (query_key, purpose, result_json, retrieved_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(query_key) DO UPDATE SET \
         purpose = excluded.purpose, result_json = excluded.result_json, \
         retrieved_at = excluded.retrieved_at, expires_at = excluded.expires_at",
        params![
            result.query_key,
            result.purpose.as_str(),
            json,
            iso(retrieved),
            iso(expires)
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Memory(RefCell<Option<(ResearchResult, Timestamp)>>);
    impl Cache for Memory {
        fn get(&self, _: &str) -> Option<(ResearchResult, Timestamp)> {
            self.0.borrow().clone()
        }
        fn put(&self, result: &ResearchResult) {
            let expires = Timestamp::from_str(&result.documents[0].citation.expires_at).unwrap();
            *self.0.borrow_mut() = Some((result.clone(), expires));
        }
    }

    struct Pages {
        body: &'static str,
        calls: Cell<usize>,
        fail: bool,
    }
    impl Fetch for Pages {
        fn get(&self, url: &str, provider: Provider) -> std::result::Result<Fetched, String> {
            assert!(allowed(url, provider));
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Err("network_unavailable".into());
            }
            Ok(Fetched {
                url: url.into(),
                content_type: "application/json; charset=utf-8".into(),
                body: self.body.as_bytes().to_vec(),
            })
        }
    }

    const ALBERT: &str = r#"{"batchcomplete": true, "query": {"pages": [
        {"pageid": 1, "title": "Albert Heijn", "fullurl": "https://en.wikipedia.org/wiki/Albert_Heijn",
         "extract": "Albert Heijn is the largest  Dutch supermarket chain."},
        {"pageid": 2, "title": "Albert Heijn Jr.", "fullurl": "https://en.wikipedia.org/wiki/AHJ",
         "extract": "A person."}]}}"#;

    #[test]
    fn merchant_research_keeps_the_exact_title_and_caches() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let cache = Memory::default();
        let pages = Pages {
            body: ALBERT,
            calls: Cell::new(0),
            fail: false,
        };
        let r = merchant(true, &cache, &pages, "Albert Heijn", None, now);
        assert_eq!(r.research.status, ResearchStatus::Live);
        assert_eq!(r.research.documents.len(), 1);
        let doc = &r.research.documents[0];
        assert_eq!(
            doc.excerpt,
            "Albert Heijn is the largest Dutch supermarket chain."
        );
        assert_eq!(doc.citation.publisher, "Wikipedia");
        let again = merchant(true, &cache, &pages, "Albert Heijn", None, now);
        assert_eq!(again.research.status, ResearchStatus::FreshCache);
        assert_eq!(pages.calls.get(), 1);
        let later = now.checked_add(SignedDuration::from_hours(25)).unwrap();
        let down = Pages {
            body: "",
            calls: Cell::new(0),
            fail: true,
        };
        let stale = merchant(true, &cache, &down, "Albert Heijn", None, later);
        assert_eq!(stale.research.status, ResearchStatus::StaleCache);
        assert_eq!(
            stale.research.error_code.as_deref(),
            Some("network_unavailable")
        );
        let off = merchant(false, &cache, &down, "Albert Heijn", None, later);
        assert_eq!(off.research.status, ResearchStatus::Disabled);
        assert_eq!(down.calls.get(), 1);
    }

    #[test]
    fn ambiguous_or_foreign_results_are_refused() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let pages = Pages {
            body: ALBERT,
            calls: Cell::new(0),
            fail: false,
        };
        let r = merchant(true, &Memory::default(), &pages, "Albert", None, now);
        assert_eq!(r.research.status, ResearchStatus::Abstained);
        assert_eq!(
            r.research.error_code.as_deref(),
            Some("ambiguous_public_identity")
        );
        let foreign = Pages {
            body: r#"{"query": {"pages": [{"title": "X", "extract": "y", "fullurl": "https://evil.example/x"}]}}"#,
            calls: Cell::new(0),
            fail: false,
        };
        let r = merchant(true, &Memory::default(), &foreign, "X Corp", None, now);
        assert_eq!(r.research.status, ResearchStatus::Unavailable);
        assert_eq!(r.research.error_code.as_deref(), Some("citation_rejected"));
    }

    #[test]
    fn only_public_names_and_exact_hosts() {
        assert_eq!(public_name(" Albert Heijn ").unwrap(), "Albert Heijn");
        for bad in [
            "a",
            "x@y.com",
            "www.shop.nl",
            "NL91ABNA0417164300",
            "Shop 12345",
            "rm -rf *",
        ] {
            assert!(public_name(bad).is_err(), "{bad}");
        }
        assert!(allowed(
            "https://en.wikipedia.org/wiki/X",
            Provider::Wikipedia
        ));
        assert!(!allowed(
            "http://en.wikipedia.org/wiki/X",
            Provider::Wikipedia
        ));
        assert!(!allowed(
            "https://en.wikipedia.org:8443/x",
            Provider::Wikipedia
        ));
        assert!(!allowed("https://www.sec.gov/x", Provider::Wikipedia));
        let q = Query::merchant("Café & Co", None);
        assert_eq!(
            q.url().unwrap(),
            "https://en.wikipedia.org/w/api.php?action=query&exintro=1&explaintext=1&format=json&formatversion=2&generator=search&gsrlimit=3&gsrsearch=Caf%C3%A9+%26+Co&inprop=url&maxlag=5&prop=extracts%7Cinfo"
        );
        assert_eq!(q.key().len(), 64);
    }

    #[test]
    fn parses_sec_atom_feeds() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let atom = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom">
            <entry><title>10-K</title><summary> Annual report </summary>
            <link href="https://www.sec.gov/Archives/x.htm"/></entry></feed>"#;
        let docs = parse(
            Provider::SecEdgar,
            &Fetched {
                url: "https://www.sec.gov/cgi-bin/browse-edgar".into(),
                content_type: "application/atom+xml".into(),
                body: atom.as_bytes().to_vec(),
            },
            now,
        )
        .unwrap();
        assert_eq!(docs[0].excerpt, "Annual report");
        assert_eq!(
            market_identifier("aapl", MarketIdentifierKind::Ticker).unwrap(),
            "AAPL"
        );
        assert!(
            Query::market(
                "US0378331005",
                MarketIdentifierKind::Isin,
                MarketQuestion::CompanyProfile
            )
            .url()
            .is_err()
        );
    }
}
