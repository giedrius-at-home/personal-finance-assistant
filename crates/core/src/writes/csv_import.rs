//! Bank CSV import (Python `csv_import.py` and the `web.py` routes
//! `/imports/csv/preview` and `/imports/csv/commit`). Rows are parsed and
//! compared exactly as Python did, but they are written into the local ledger
//! (transactions, postings, search row and an `import_runs` record) instead of
//! Firefly, and deduplicated against the ledger's rows.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use finance_storage::{NewTransaction, counterparty_account, insert_transaction};
use finance_types::manage::{
    CsvCommit, CsvCommitResult, CsvPreview, CsvPreviewRow, CsvRejectedRow, CsvRowStatus,
};
use jiff::civil::Date;
use rusqlite::{Connection, params};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use super::{Writer, invalid};
use crate::Result;

pub const MAX_CSV_BYTES: usize = 5 * 1024 * 1024;
const MAX_CSV_ROWS: usize = 10_000;
const MAX_PREVIEW_ROWS: usize = 100;
const PREVIEW_TTL: Duration = Duration::from_secs(30 * 60);
const DELIMITERS: [char; 4] = [',', ';', '\t', '|'];
const GENERIC_LABEL: &str = "Connected bank CSV";

/// A normalized row, kept only in the in-memory preview store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedRow {
    pub line_number: u32,
    pub booked_on: Date,
    pub amount: Decimal,
    pub currency: String,
    pub description: String,
    pub external_reference: Option<String>,
    pub counterparty_iban: Option<String>,
}

/// An own asset account an import can target (Firefly's asset accounts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub id: i64,
    pub name: String,
    pub iban: Option<String>,
    pub active: bool,
    pub currency: String,
}

/// The ledger's asset accounts; `currencies` maps a ledger id to the
/// account's currency (the one most of its postings use).
pub fn targets(conn: &Connection, currencies: &HashMap<i64, String>) -> Result<Vec<Target>> {
    let mut stmt = conn
        .prepare("SELECT id, name, iban, active FROM accounts WHERE kind = 'asset' ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, bool>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, iban, active) = row?;
        out.push(Target {
            id,
            currency: currencies.get(&id).cloned().unwrap_or_else(|| "EUR".into()),
            name,
            iban,
            active,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- parsing

const DATE: &str = "invalid date";
const AMOUNT: &str = "invalid amount";
const CURRENCY: &str = "invalid currency";
const DESCRIPTION: &str = "missing description";
const ROW: &str = "invalid row";

/// Header aliases per role, in Python's order.
const HEADER_ALIASES: [(&str, &[&str]); 8] = [
    (
        "date",
        &[
            "date",
            "booking date",
            "book date",
            "transaction date",
            "completed date",
            "datum",
            "boekdatum",
            "data",
        ],
    ),
    (
        "amount",
        &[
            "amount",
            "transaction amount",
            "value",
            "bedrag",
            "sum",
            "suma",
        ],
    ),
    ("debit", &["debit", "debit amount", "af", "expenses"]),
    ("credit", &["credit", "credit amount", "bij", "income"]),
    (
        "currency",
        &["currency", "currency code", "valuta", "valiuta", "ccy"],
    ),
    (
        "description",
        &[
            "description",
            "details",
            "merchant",
            "recipient",
            "beneficiary",
            "omschrijving",
            "naam omschrijving",
            "memo",
        ],
    ),
    (
        "reference",
        &[
            "reference",
            "transaction id",
            "external id",
            "payment reference",
            "kenmerk",
        ],
    ),
    (
        "iban",
        &[
            "counterparty iban",
            "iban",
            "tegenrekening iban",
            "other account",
        ],
    ),
];

/// Python `str.casefold` for the text banks export.
fn casefold(value: &str) -> String {
    value.to_lowercase().replace('ß', "ss")
}

fn normalize_header(value: &str) -> String {
    let folded = casefold(value);
    let mut out = String::with_capacity(folded.len());
    let mut gap = false;
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

fn normalize_description(value: &str) -> String {
    casefold(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_iban(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Python `Path(filename).stem`.
fn stem(path: &str) -> &str {
    let name = file_name(path);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// A display-only provider label from the file name.
pub fn detect_bank_label(filename: &str) -> &'static str {
    let normalized = normalize_header(stem(filename));
    [
        ("abn", "ABN AMRO"),
        ("revolut", "Revolut"),
        ("swedbank", "Swedbank"),
    ]
    .into_iter()
    .find(|(marker, _)| normalized.contains(marker))
    .map_or(GENERIC_LABEL, |(_, label)| label)
}

/// The bank an account belongs to, by the same markers.
pub fn bank_of(account_name: &str) -> Option<&'static str> {
    let normalized = normalize_header(account_name);
    ["ABN AMRO", "Revolut", "Swedbank"]
        .into_iter()
        .find(|label| normalized.contains(&normalize_header(label)))
}

/// The opaque, deterministic id an imported row is stored with.
pub fn csv_external_id(row: &ParsedRow, account_id: &str, bank_label: &str) -> String {
    let reference = row.external_reference.clone().unwrap_or_else(|| {
        [
            row.booked_on.to_string(),
            row.amount.to_string(),
            row.currency.clone(),
            normalize_description(&row.description),
            normalize_iban(row.counterparty_iban.as_deref().unwrap_or("")),
        ]
        .join("|")
    });
    let digest =
        Sha256::digest(format!("v1|{}|{account_id}|{reference}", casefold(bank_label)).as_bytes());
    format!("pfa-csv-v1-{}", hex(&digest))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Python `csv.Sniffer().sniff(sample, delimiters=",;\t|")`: the delimiter
/// and whether spaces after it are skipped.
fn sniff(sample: &str) -> Option<(char, bool)> {
    quoted_delimiter(sample).or_else(|| frequent_delimiter(sample))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn delimiter_like(c: char) -> bool {
    !is_word(c) && c != '\n' && c != '"' && c != '\''
}

/// Sniffer `_guess_quote_and_delimiter`: delimiters seen next to quoted fields.
fn quoted_delimiter(sample: &str) -> Option<(char, bool)> {
    let chars: Vec<char> = sample.chars().collect();
    let n = chars.len();
    let quote = |c: char| c == '"' || c == '\'';
    // (delimiter, space after it) per match, for each of Python's patterns in turn.
    let mut found: Vec<(Option<char>, bool)> = Vec::new();
    // 1: D ␠? Q .*? Q D
    let mut i = 0;
    while i < n {
        let d = chars[i];
        let mut next = None;
        if delimiter_like(d) {
            for space in [true, false] {
                let q_at = i + 1 + usize::from(space);
                if space && chars.get(i + 1) != Some(&' ') {
                    continue;
                }
                let Some(&q) = chars.get(q_at) else { continue };
                if !quote(q) {
                    continue;
                }
                if let Some(j) =
                    (q_at + 1..n.saturating_sub(1)).find(|&j| chars[j] == q && chars[j + 1] == d)
                {
                    found.push((Some(d), space));
                    next = Some(j + 2);
                    break;
                }
            }
        }
        i = next.unwrap_or(i + 1);
    }
    // 2: (^|\n) Q .*? Q D ␠?
    if found.is_empty() {
        let mut i = 0;
        while i < n {
            let start = if i == 0 && quote(chars[0]) {
                Some(0)
            } else if chars[i] == '\n' && i + 1 < n && quote(chars[i + 1]) {
                Some(i + 1)
            } else {
                None
            };
            let mut next = None;
            if let Some(s) = start {
                let q = chars[s];
                if let Some(j) = (s + 1..n.saturating_sub(1))
                    .find(|&j| chars[j] == q && delimiter_like(chars[j + 1]))
                {
                    let space = chars.get(j + 2) == Some(&' ');
                    found.push((Some(chars[j + 1]), space));
                    next = Some(j + 2 + usize::from(space));
                }
            }
            i = next.unwrap_or(i + 1);
        }
    }
    // 3: D ␠? Q .*? Q ($|\n)
    if found.is_empty() {
        let mut i = 0;
        while i < n {
            let d = chars[i];
            let mut next = None;
            if delimiter_like(d) {
                for space in [true, false] {
                    let q_at = i + 1 + usize::from(space);
                    if space && chars.get(i + 1) != Some(&' ') {
                        continue;
                    }
                    let Some(&q) = chars.get(q_at) else { continue };
                    if !quote(q) {
                        continue;
                    }
                    if let Some(j) = (q_at + 1..n)
                        .find(|&j| chars[j] == q && (j + 1 == n || chars[j + 1] == '\n'))
                    {
                        found.push((Some(d), space));
                        next = Some(j + 1 + usize::from(j + 1 < n));
                        break;
                    }
                }
            }
            i = next.unwrap_or(i + 1);
        }
    }
    // Pattern 4 has no delimiter, so it never decides one.
    let spaces = found.iter().filter(|(_, space)| *space).count();
    let mut counts: Vec<(char, usize)> = Vec::new();
    for (d, _) in found {
        let Some(d) = d.filter(|d| DELIMITERS.contains(d)) else {
            continue;
        };
        match counts.iter_mut().find(|(c, _)| *c == d) {
            Some(entry) => entry.1 += 1,
            None => counts.push((d, 1)),
        }
    }
    // Python's `max` keeps the first of equal counts.
    let best = counts
        .iter()
        .fold(None::<&(char, usize)>, |best, c| match best {
            Some(b) if b.1 >= c.1 => Some(b),
            _ => Some(c),
        })?;
    Some((best.0, best.1 == spaces))
}

/// Sniffer `_guess_delimiter`: the delimiter whose count per line is most
/// consistent.
fn frequent_delimiter(sample: &str) -> Option<(char, bool)> {
    let data: Vec<&str> = sample.split('\n').filter(|l| !l.is_empty()).collect();
    let first = data.first().copied().unwrap_or("");
    let skip = |d: char| first.matches(d).count() == first.matches(&format!("{d} ")).count();
    let chunk = data.len().min(10);
    if chunk == 0 {
        return None;
    }
    let mut frequency: HashMap<char, Vec<(usize, usize)>> = HashMap::new();
    let mut delims: HashMap<char, (usize, i64)> = HashMap::new();
    let (mut start, mut end, mut iteration) = (0, chunk, 0);
    while start < data.len() {
        iteration += 1;
        for line in &data[start..end.min(data.len())] {
            for d in DELIMITERS {
                let count = line.matches(d).count();
                let meta = frequency.entry(d).or_default();
                match meta.iter_mut().find(|(f, _)| *f == count) {
                    Some(entry) => entry.1 += 1,
                    None => meta.push((count, 1)),
                }
            }
        }
        let mut modes: Vec<(char, (usize, i64))> = Vec::new();
        for d in DELIMITERS {
            let Some(items) = frequency.get(&d) else {
                continue;
            };
            if items.len() == 1 && items[0].0 == 0 {
                continue;
            }
            // Python's `max` returns the first of equal counts.
            let mode = items
                .iter()
                .fold(None::<&(usize, usize)>, |best, x| match best {
                    Some(b) if b.1 >= x.1 => Some(b),
                    _ => Some(x),
                })
                .copied()
                .unwrap_or((0, 0));
            let rest: usize = items
                .iter()
                .filter(|x| **x != mode)
                .map(|x| x.1)
                .sum::<usize>();
            let rest = if items.len() > 1 { rest } else { 0 };
            modes.push((d, (mode.0, mode.1 as i64 - rest as i64)));
        }
        let total = (chunk * iteration).min(data.len()) as f64;
        let mut consistency = 1.0_f64;
        while delims.is_empty() && consistency >= 0.9 {
            for (d, v) in &modes {
                if v.0 > 0 && v.1 > 0 && (v.1 as f64 / total) >= consistency {
                    delims.insert(*d, *v);
                }
            }
            consistency -= 0.01;
        }
        if delims.len() == 1 {
            let d = *delims.keys().next()?;
            return Some((d, skip(d)));
        }
        start = end;
        end += chunk;
    }
    if delims.is_empty() {
        return None;
    }
    for d in [',', '\t', ';'] {
        if delims.contains_key(&d) {
            return Some((d, skip(d)));
        }
    }
    let d = delims
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(a.0.cmp(b.0)))
        .map(|(d, _)| *d)?;
    Some((d, skip(d)))
}

/// Python's `csv.reader` (non-strict, `"` quotes, doubled quotes) over text
/// read with `newline=""`.
fn read_records(text: &str, delimiter: char, skip_space: bool) -> Vec<Vec<String>> {
    #[derive(PartialEq)]
    enum State {
        StartRecord,
        StartField,
        InField,
        InQuoted,
        QuoteInQuoted,
        EatCrNl,
    }
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut state = State::StartRecord;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if state == State::EatCrNl {
            state = State::StartRecord;
            if c == '\n' {
                continue;
            }
        }
        if state == State::StartRecord {
            if c == '\n' || c == '\r' {
                records.push(Vec::new());
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }
            state = State::StartField;
        }
        match state {
            State::StartField => {
                if c == '\n' || c == '\r' {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                    state = if c == '\r' {
                        State::EatCrNl
                    } else {
                        State::StartRecord
                    };
                } else if c == '"' {
                    state = State::InQuoted;
                } else if c == ' ' && skip_space {
                } else if c == delimiter {
                    record.push(std::mem::take(&mut field));
                } else {
                    field.push(c);
                    state = State::InField;
                }
            }
            State::InField => {
                if c == '\n' || c == '\r' {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                    state = if c == '\r' {
                        State::EatCrNl
                    } else {
                        State::StartRecord
                    };
                } else if c == delimiter {
                    record.push(std::mem::take(&mut field));
                    state = State::StartField;
                } else {
                    field.push(c);
                }
            }
            State::InQuoted => {
                if c == '"' {
                    state = State::QuoteInQuoted;
                } else {
                    field.push(c);
                }
            }
            State::QuoteInQuoted => {
                if c == '"' {
                    field.push('"');
                    state = State::InQuoted;
                } else if c == delimiter {
                    record.push(std::mem::take(&mut field));
                    state = State::StartField;
                } else if c == '\n' || c == '\r' {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                    state = if c == '\r' {
                        State::EatCrNl
                    } else {
                        State::StartRecord
                    };
                } else {
                    field.push(c);
                    state = State::InField;
                }
            }
            State::StartRecord | State::EatCrNl => {}
        }
    }
    if !matches!(state, State::StartRecord | State::EatCrNl) {
        record.push(field);
        records.push(record);
    }
    records
}

/// Python `_parse_date`.
fn parse_date(value: &str) -> Option<Date> {
    let cleaned = value.trim();
    if cleaned.contains('T') {
        let (day, time) = cleaned.split_once('T')?;
        if let Some(date) = iso_date(day)
            && !time.is_empty()
            && time
                .chars()
                .all(|c| c.is_ascii_digit() || ":.,+-Z".contains(c))
        {
            return Some(date);
        }
    }
    // %Y-%m-%d, %d-%m-%Y, %d/%m/%Y, %Y/%m/%d, %d.%m.%Y
    [
        ('-', true),
        ('-', false),
        ('/', false),
        ('/', true),
        ('.', false),
    ]
    .into_iter()
    .find_map(|(sep, year_first)| {
        let parts: Vec<&str> = cleaned.split(sep).collect();
        let [a, b, c] = parts.as_slice() else {
            return None;
        };
        let (y, m, d) = if year_first { (a, b, c) } else { (c, b, a) };
        let ok = |s: &str, max: usize| {
            (1..=max).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit())
        };
        if y.len() != 4 || !ok(y, 4) || !ok(m, 2) || !ok(d, 2) {
            return None;
        }
        Date::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?).ok()
    })
}

fn iso_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    Date::from_str(text).ok()
}

/// Python `_parse_decimal`: thousands separators, decimal commas and
/// accounting parentheses.
fn parse_decimal(value: &str) -> Option<Decimal> {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| c.is_ascii_digit() || ",.()-+".contains(*c))
        .collect();
    let negative_parentheses = cleaned.starts_with('(') && cleaned.ends_with(')');
    let mut cleaned = cleaned.trim_matches(['(', ')']).to_owned();
    if cleaned.contains(',') && cleaned.contains('.') {
        let decimal_mark = if cleaned.rfind(',') > cleaned.rfind('.') {
            ","
        } else {
            "."
        };
        let thousands = if decimal_mark == "," { "." } else { "," };
        cleaned = cleaned.replace(thousands, "").replace(decimal_mark, ".");
    } else if let Some((_, tail)) = cleaned.rsplit_once(',') {
        let mark = if tail.len() <= 2 { "." } else { "" };
        cleaned = cleaned.replace(',', mark);
    }
    // Python `Decimal(text)` for text made of digits, signs and points.
    let (negative, digits) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.strip_prefix('+').unwrap_or(&cleaned)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .chars()
            .chain(fraction.chars())
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let text = format!(
        "{}{}{}{}",
        if negative { "-" } else { "" },
        if whole.is_empty() { "0" } else { whole },
        if fraction.is_empty() { "" } else { "." },
        fraction
    );
    let result = Decimal::from_str(&text).ok()?;
    Some(if negative_parentheses {
        -result.abs()
    } else {
        result
    })
}

fn parse_row(
    value: &dyn Fn(&str) -> String,
    line_number: u32,
) -> std::result::Result<ParsedRow, &'static str> {
    let mut amount_text = value("amount");
    if amount_text.is_empty() {
        let (debit, credit) = (value("debit"), value("credit"));
        if debit.is_empty() == credit.is_empty() {
            return Err(AMOUNT);
        }
        amount_text = if debit.is_empty() {
            credit
        } else {
            format!("-{debit}")
        };
    }
    let description = value("description");
    if description.is_empty() {
        return Err(DESCRIPTION);
    }
    let booked_on = parse_date(&value("date")).ok_or(DATE)?;
    let amount = parse_decimal(&amount_text).ok_or(AMOUNT)?;
    let currency = {
        let raw = value("currency");
        let raw = if raw.is_empty() {
            "EUR".to_owned()
        } else {
            raw
        };
        raw.trim().to_uppercase()
    };
    let reference = Some(value("reference")).filter(|v| !v.is_empty());
    let iban = Some(value("iban")).filter(|v| !v.is_empty());
    // The model's checks, reported in the order Python names them.
    if amount.is_zero() {
        return Err(AMOUNT);
    }
    if currency.chars().count() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(CURRENCY);
    }
    if description.chars().count() > 1024 {
        return Err(DESCRIPTION);
    }
    if reference.as_ref().is_some_and(|r| r.chars().count() > 512)
        || iban.as_ref().is_some_and(|i| i.chars().count() > 64)
    {
        return Err(ROW);
    }
    Ok(ParsedRow {
        line_number,
        booked_on,
        amount,
        currency,
        description,
        external_reference: reference,
        counterparty_iban: iban,
    })
}

/// Parse a bank export by its column names; unreadable rows are kept only
/// as line numbers and reasons.
pub fn parse_bank_csv(
    content: &str,
    filename: &str,
) -> Result<(Vec<ParsedRow>, Vec<CsvRejectedRow>)> {
    if content.is_empty() || content.len() > MAX_CSV_BYTES {
        return Err(invalid("CSV must be non-empty and no larger than 5 MiB."));
    }
    let lower = file_name(filename).to_lowercase();
    if !(lower.ends_with(".csv") || lower.ends_with(".txt")) {
        return Err(invalid("Upload must be a CSV or delimited text file."));
    }
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let sample: String = text.chars().take(8192).collect();
    let (delimiter, skip_space) =
        sniff(&sample).ok_or_else(|| invalid("CSV delimiter could not be detected."))?;
    let mut records = read_records(text, delimiter, skip_space).into_iter();
    let header = records.next().unwrap_or_default();
    if header.is_empty() {
        return Err(invalid("CSV must contain a header row."));
    }
    // A dict per row: the last column of a repeated name wins.
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, name) in header.iter().enumerate() {
        index.insert(name.as_str(), i);
    }
    let mut columns: HashMap<&str, usize> = HashMap::new();
    for name in &header {
        let normalized = normalize_header(name);
        for (role, aliases) in HEADER_ALIASES {
            if aliases.contains(&normalized.as_str()) && !columns.contains_key(role) {
                columns.insert(role, index[name.as_str()]);
            }
        }
    }
    if !columns.contains_key("date")
        || !(columns.contains_key("amount")
            || columns.contains_key("debit")
            || columns.contains_key("credit"))
    {
        return Err(invalid("CSV needs recognizable date and amount columns."));
    }
    if !columns.contains_key("description") {
        return Err(invalid("CSV needs a recognizable description column."));
    }
    let mut rows = Vec::new();
    let mut rejected = Vec::new();
    for (i, record) in records.filter(|r| !r.is_empty()).enumerate() {
        let line_number = i + 2;
        if line_number > MAX_CSV_ROWS + 1 {
            return Err(invalid("CSV contains more than 10000 data rows."));
        }
        let line_number = line_number as u32;
        let value = |role: &str| -> String {
            columns
                .get(role)
                .and_then(|&c| record.get(c))
                .map(|v| v.trim().to_owned())
                .unwrap_or_default()
        };
        match parse_row(&value, line_number) {
            Ok(row) => rows.push(row),
            Err(reason) => rejected.push(CsvRejectedRow {
                line_number,
                reason: reason.to_owned(),
            }),
        }
    }
    if rows.is_empty() {
        return Err(invalid("CSV contains no valid transaction rows."));
    }
    Ok((rows, rejected))
}

// ---------------------------------------------------------- comparison

fn one_account<'a>(
    accounts: &'a [Target],
    account_id: &str,
    bank_label: &str,
) -> Result<&'a Target> {
    let matches: Vec<&Target> = if account_id == "auto" {
        let marker = normalize_header(bank_label);
        accounts
            .iter()
            .filter(|a| {
                marker != "connected bank csv" && normalize_header(&a.name).contains(&marker)
            })
            .collect()
    } else {
        accounts
            .iter()
            .filter(|a| a.id.to_string() == account_id)
            .collect()
    };
    match matches.as_slice() {
        [one] if one.active => Ok(one),
        _ => Err(invalid(
            "Bank account could not be detected uniquely; choose it explicitly.",
        )),
    }
}

type Exact = (Date, Decimal, String, String);
type Signature = (Date, Decimal, String);
/// Exact fingerprints, stored ids and amount signatures with their counts.
type Existing = (
    HashMap<Exact, usize>,
    HashSet<String>,
    HashMap<Signature, usize>,
);

/// The ledger rows of `account` booked between `start` and `end`: exact
/// fingerprints, stored ids and amount signatures (Python
/// `_existing_fingerprints` over Firefly's listing).
fn existing(conn: &Connection, account: i64, start: Date, end: Date) -> Result<Existing> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.date, p.amount, p.currency, t.description, t.external_id
         FROM postings p JOIN transactions t ON t.id = p.transaction_id
         WHERE p.account_id = ?1 AND t.date BETWEEN ?2 AND ?3",
    )?;
    let rows = stmt.query_map(params![account, start.to_string(), end.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut exact = HashMap::new();
    let mut identifiers = HashSet::new();
    let mut amounts = HashMap::new();
    for row in rows {
        let (date, amount, currency, description, external_id) = row?;
        let date =
            Date::from_str(&date).map_err(|_| crate::Error::Invalid(format!("date {date:?}")))?;
        let amount = Decimal::from_str(&amount)
            .map_err(|_| crate::Error::Invalid(format!("amount {amount:?}")))?
            .normalize();
        *amounts.entry((date, amount, currency.clone())).or_insert(0) += 1;
        *exact
            .entry((date, amount, currency, normalize_description(&description)))
            .or_insert(0) += 1;
        if let Some(id) = external_id.filter(|i| !i.is_empty()) {
            identifiers.insert(id);
        }
    }
    Ok((exact, identifiers, amounts))
}

/// Python `_classify_rows`.
fn classify(
    conn: &Connection,
    rows: &[ParsedRow],
    account: &Target,
    bank_label: &str,
) -> Result<Vec<CsvRowStatus>> {
    let start = rows.iter().map(|r| r.booked_on).min();
    let end = rows.iter().map(|r| r.booked_on).max();
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(Vec::new());
    };
    let (existing_exact, existing_ids, existing_amounts) = existing(conn, account.id, start, end)?;
    let account_id = account.id.to_string();
    let mut statuses = Vec::with_capacity(rows.len());
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_unreferenced: HashMap<Exact, usize> = HashMap::new();
    let mut used_exact: HashMap<Exact, usize> = HashMap::new();
    for row in rows {
        let external_id = csv_external_id(row, &account_id, bank_label);
        let amount = row.amount.normalize();
        let exact: Exact = (
            row.booked_on,
            amount,
            row.currency.clone(),
            normalize_description(&row.description),
        );
        let signature: Signature = (row.booked_on, amount, row.currency.clone());
        let provider = row.external_reference.as_deref();
        let status = if existing_ids.contains(&external_id)
            || provider.is_some_and(|p| existing_ids.contains(p))
        {
            CsvRowStatus::Duplicate
        } else if seen_ids.contains(&external_id) {
            if provider.is_some() {
                CsvRowStatus::Duplicate
            } else {
                CsvRowStatus::PossibleDuplicate
            }
        } else if used_exact.get(&exact).copied().unwrap_or(0)
            < existing_exact.get(&exact).copied().unwrap_or(0)
        {
            *used_exact.entry(exact.clone()).or_insert(0) += 1;
            CsvRowStatus::Duplicate
        } else if existing_amounts.get(&signature).copied().unwrap_or(0) > 0
            || (provider.is_none() && seen_unreferenced.get(&exact).copied().unwrap_or(0) > 0)
        {
            CsvRowStatus::PossibleDuplicate
        } else {
            CsvRowStatus::New
        };
        statuses.push(status);
        seen_ids.insert(external_id);
        if provider.is_none() {
            *seen_unreferenced.entry(exact).or_insert(0) += 1;
        }
    }
    Ok(statuses)
}

// ------------------------------------------------------------- staging

#[derive(Clone, Debug)]
struct Staged {
    preview: CsvPreview,
    account_id: i64,
    rows: Vec<ParsedRow>,
    statuses: Vec<CsvRowStatus>,
    created: Instant,
}

/// Short-lived, process-local previews; uploads are never written to disk.
#[derive(Debug, Default)]
struct PreviewStore {
    items: HashMap<String, Staged>,
    in_progress: HashSet<String>,
}

static PREVIEWS: LazyLock<Mutex<PreviewStore>> = LazyLock::new(Mutex::default);

fn previews() -> Result<std::sync::MutexGuard<'static, PreviewStore>> {
    PREVIEWS
        .lock()
        .map_err(|_| crate::Error::Invalid("preview store poisoned".into()))
}

fn begin(preview_id: &str) -> Result<Staged> {
    let mut store = previews()?;
    let now = Instant::now();
    let PreviewStore { items, in_progress } = &mut *store;
    items
        .retain(|key, s| now.duration_since(s.created) <= PREVIEW_TTL || in_progress.contains(key));
    let staged = items
        .get(preview_id)
        .cloned()
        .ok_or_else(|| invalid("CSV preview is missing or expired."))?;
    if !in_progress.insert(preview_id.to_owned()) {
        return Err(invalid("CSV preview import is already in progress."));
    }
    Ok(staged)
}

fn release(preview_id: &str, completed: bool) {
    if let Ok(mut store) = previews() {
        store.in_progress.remove(preview_id);
        if completed {
            store.items.remove(preview_id);
        }
    }
}

fn new_preview_id(content: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seed = format!("{now}|{n}|{}|{:p}", content.len(), &COUNTER);
    hex(&Sha256::digest(
        [seed.as_bytes(), &Sha256::digest(content.as_bytes())].concat(),
    ))
}

/// Parse, compare and stage `content`; the returned id commits exactly it.
pub fn preview(
    conn: &Connection,
    accounts: &[Target],
    content: &str,
    filename: &str,
    account_id: &str,
) -> Result<CsvPreview> {
    if filename.chars().count() > 255 {
        return Err(invalid("Invalid CSV filename."));
    }
    let (rows, rejected) = parse_bank_csv(content, filename)?;
    let bank_label = detect_bank_label(filename);
    let account = one_account(accounts, account_id, bank_label)?;
    if bank_label != GENERIC_LABEL
        && !normalize_header(&account.name).contains(&normalize_header(bank_label))
    {
        return Err(invalid("CSV bank does not match the selected account."));
    }
    if rows.iter().any(|r| r.currency != account.currency) {
        return Err(invalid("CSV currency does not match the selected account."));
    }
    let statuses = classify(conn, &rows, account, bank_label)?;
    let count = |s: CsvRowStatus| statuses.iter().filter(|x| **x == s).count() as u32;
    let preview = CsvPreview {
        preview_id: new_preview_id(content),
        bank_label: bank_label.to_owned(),
        account_name: account.name.clone(),
        row_count: (rows.len() + rejected.len()) as u32,
        new_count: count(CsvRowStatus::New),
        duplicate_count: count(CsvRowStatus::Duplicate),
        possible_duplicate_count: count(CsvRowStatus::PossibleDuplicate),
        rejected_count: rejected.len() as u32,
        committable: rejected.is_empty() && count(CsvRowStatus::PossibleDuplicate) == 0,
        rows: rows
            .iter()
            .zip(&statuses)
            .take(MAX_PREVIEW_ROWS)
            .map(|(r, s)| CsvPreviewRow {
                line_number: r.line_number,
                booked_on: r.booked_on.to_string(),
                amount: r.amount,
                currency: r.currency.clone(),
                description: r.description.clone(),
                status: *s,
            })
            .collect(),
        rejected_rows: rejected.into_iter().take(MAX_PREVIEW_ROWS).collect(),
    };
    previews()?.items.insert(
        preview.preview_id.clone(),
        Staged {
            preview: preview.clone(),
            account_id: account.id,
            rows,
            statuses,
            created: Instant::now(),
        },
    );
    Ok(preview)
}

/// Import the staged preview's new rows into the ledger, if the ledger still
/// matches what the preview showed.
pub fn commit(w: &Writer<'_>, accounts: &[Target], request: &CsvCommit) -> Result<CsvCommitResult> {
    let staged = begin(&request.preview_id)?;
    let result = commit_staged(w, accounts, &staged, request.skip_flagged);
    release(&request.preview_id, result.is_ok());
    result
}

fn commit_staged(
    w: &Writer<'_>,
    accounts: &[Target],
    staged: &Staged,
    skip_flagged: bool,
) -> Result<CsvCommitResult> {
    if !staged.preview.committable && !skip_flagged {
        return Err(invalid(
            "CSV preview contains rejected or possible duplicate rows.",
        ));
    }
    let label = staged.preview.bank_label.as_str();
    let account = one_account(accounts, &staged.account_id.to_string(), label)?;
    let current = classify(&w.tx, &staged.rows, account, label)?;
    let safe = staged.statuses.iter().zip(&current).all(|(original, now)| {
        original == now || (*original == CsvRowStatus::New && *now == CsvRowStatus::Duplicate)
    });
    if !safe {
        return Err(invalid(
            "The ledger changed after the CSV preview was created. Prepare a new preview.",
        ));
    }
    let account_id = account.id.to_string();
    let mut imported = 0_u32;
    for (row, status) in staged.rows.iter().zip(&current) {
        if *status != CsvRowStatus::New {
            continue;
        }
        let other = row.counterparty_iban.as_deref().and_then(|iban| {
            let iban = normalize_iban(iban);
            let found: Vec<&Target> = accounts
                .iter()
                .filter(|a| a.iban.as_deref().is_some_and(|i| normalize_iban(i) == iban))
                .collect();
            match found.as_slice() {
                [one] => Some(*one),
                _ => None,
            }
        });
        let outgoing = row.amount.is_sign_negative();
        let (kind, source, destination) = match other {
            Some(other) if other.id != account.id => (
                "transfer",
                if outgoing { account.id } else { other.id },
                if outgoing { other.id } else { account.id },
            ),
            _ if outgoing => (
                "withdrawal",
                account.id,
                counterparty_account(&w.tx, &row.description, "expense")?,
            ),
            _ => (
                "deposit",
                counterparty_account(&w.tx, &row.description, "revenue")?,
                account.id,
            ),
        };
        let external_id = csv_external_id(row, &account_id, label);
        let date = row.booked_on.to_string();
        insert_transaction(
            &w.tx,
            &NewTransaction {
                kind,
                date: &date,
                description: &row.description,
                source_account_id: source,
                destination_account_id: destination,
                amount: row.amount.abs(),
                currency: &row.currency,
                external_id: Some(&external_id),
                internal_reference: None,
                notes: None,
                tags: &[],
            },
        )?;
        imported += 1;
    }
    w.record(
        crate::manage::CSV_SOURCE,
        &serde_json::json!({
            "account_id": account.id,
            "bank": label,
            "imported": imported,
            "skipped": staged.rows.len() as u32 - imported,
            "rejected": staged.preview.rejected_count,
        }),
    )?;
    Ok(CsvCommitResult {
        imported,
        refreshed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amounts_like_python() {
        let d = |s: &str| parse_decimal(s).map(|d| d.to_string());
        assert_eq!(d("1.234,56"), Some("1234.56".into()));
        assert_eq!(d("1,234.56"), Some("1234.56".into()));
        assert_eq!(d("12,5"), Some("12.5".into()));
        assert_eq!(d("1,000"), Some("1000".into()));
        assert_eq!(d("(12.30)"), Some("-12.30".into()));
        assert_eq!(d("€ -2.10"), Some("-2.10".into()));
        assert_eq!(d("+5"), Some("5".into()));
        assert_eq!(d(".50"), Some("0.50".into()));
        assert_eq!(d("abc"), None);
        assert_eq!(d("1.2.3"), None);
    }

    #[test]
    fn parses_dates_like_python() {
        let d = |s: &str| parse_date(s).map(|d| d.to_string());
        assert_eq!(d("2026-10-12"), Some("2026-10-12".into()));
        assert_eq!(d("12-10-2026"), Some("2026-10-12".into()));
        assert_eq!(d("1/2/2026"), Some("2026-02-01".into()));
        assert_eq!(d("2026/10/12"), Some("2026-10-12".into()));
        assert_eq!(d("12.10.2026"), Some("2026-10-12".into()));
        assert_eq!(d("2026-10-12T10:11:12Z"), Some("2026-10-12".into()));
        assert_eq!(d("2026-02-30"), None);
        assert_eq!(d("yesterday"), None);
    }

    #[test]
    fn sniffs_delimiters() {
        assert_eq!(sniff("a,b,c\n1,2,3\n4,5,6\n").map(|s| s.0), Some(','));
        assert_eq!(sniff("a;b;c\n1;2;3\n4;5;6\n").map(|s| s.0), Some(';'));
        assert_eq!(sniff("a\tb\tc\n1\t2\t3\n").map(|s| s.0), Some('\t'));
        assert_eq!(
            sniff("Date;Amount;Description\n2026-01-01;\"1,50\";\"Shop, A\"\n").map(|s| s.0),
            Some(';')
        );
        assert_eq!(sniff("nothing here\nat all\n"), None);
    }

    #[test]
    fn reads_quoted_records() {
        let records = read_records(
            "a,b\r\n\"x, \"\"y\"\"\",2\n\n3,\"multi\nline\"\n",
            ',',
            false,
        );
        assert_eq!(
            records,
            vec![
                vec!["a".to_owned(), "b".to_owned()],
                vec!["x, \"y\"".to_owned(), "2".to_owned()],
                vec![],
                vec!["3".to_owned(), "multi\nline".to_owned()],
            ]
        );
    }

    #[test]
    fn parses_rows_and_rejections() {
        let csv = "Date,Amount,Currency,Description,Reference\n\
                   2026-10-01,-12.30,EUR,Albert Heijn,\n\
                   2026-10-02,0,EUR,Zero,\n\
                   bad,1,EUR,Shop,\n\
                   2026-10-03,5,EURO,Shop,\n\
                   2026-10-04,5,EUR,,\n\
                   \n\
                   2026-10-05,7,usd,Refund,R1\n";
        let (rows, rejected) = parse_bank_csv(csv, "revolut.csv").expect("parses");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].amount.to_string(), "-12.30");
        assert_eq!(rows[1].currency, "USD");
        assert_eq!(rows[1].line_number, 7);
        assert_eq!(rows[1].external_reference.as_deref(), Some("R1"));
        let reasons: Vec<(u32, &str)> = rejected
            .iter()
            .map(|r| (r.line_number, r.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![(3, AMOUNT), (4, DATE), (5, CURRENCY), (6, DESCRIPTION)]
        );
        assert!(parse_bank_csv(csv, "revolut.pdf").is_err());
        assert!(parse_bank_csv("Day,Value\n1,2\n", "x.csv").is_err());
    }

    #[test]
    fn debit_and_credit_columns() {
        let csv = "Datum;Af;Bij;Omschrijving\n01-10-2026;12,50;;Shop\n02-10-2026;;100;Salary\n03-10-2026;1;2;Both\n";
        let (rows, rejected) = parse_bank_csv(csv, "ABN_export.txt").expect("parses");
        assert_eq!(rows[0].amount.to_string(), "-12.50");
        assert_eq!(rows[1].amount.to_string(), "100");
        assert_eq!(rejected[0].reason, AMOUNT);
        assert_eq!(detect_bank_label("ABN_export.txt"), "ABN AMRO");
        assert_eq!(detect_bank_label("statement.csv"), GENERIC_LABEL);
    }

    #[test]
    fn external_ids_are_stable() {
        let row = ParsedRow {
            line_number: 2,
            booked_on: Date::constant(2026, 10, 1),
            amount: Decimal::new(-1230, 2),
            currency: "EUR".into(),
            description: "Albert  Heijn".into(),
            external_reference: None,
            counterparty_iban: Some("nl00 abna 1".into()),
        };
        let expected = hex(&Sha256::digest(
            b"v1|revolut|76|2026-10-01|-12.30|EUR|albert heijn|NL00ABNA1",
        ));
        assert_eq!(
            csv_external_id(&row, "76", "Revolut"),
            format!("pfa-csv-v1-{expected}")
        );
    }

    fn ledger() -> finance_storage::Store {
        let store = finance_storage::Store::open(std::path::Path::new(":memory:")).expect("store");
        store
            .connection()
            .execute_batch(
                "INSERT INTO accounts (id, name, kind, iban, active) VALUES
                    (1, 'ABN AMRO', 'asset', 'NL43ABNA0136006396', 1),
                    (76, 'Revolut', 'asset', 'NL39REVO0305618369', 1),
                    (413, 'Revolut Savings', 'asset', NULL, 1),
                    (500, 'Albert Heijn', 'expense', NULL, 1);
                 INSERT INTO transactions (id, kind, date, booked_at, description,
                    source_account_id, destination_account_id, external_id)
                    VALUES (10, 'withdrawal', '2026-10-01', '2026-10-01T00:00:00', 'Albert Heijn', 76, 500, 'bank-1');
                 INSERT INTO postings (transaction_id, account_id, amount, currency) VALUES
                    (10, 76, '-12.300000', 'EUR'), (10, 500, '12.300000', 'EUR');",
            )
            .expect("rows");
        store
    }

    fn accounts(store: &finance_storage::Store) -> Vec<Target> {
        targets(store.connection(), &HashMap::new()).expect("targets")
    }

    #[test]
    fn previews_and_commits_into_the_ledger() {
        let mut store = ledger();
        let targets = accounts(&store);
        let csv = "Date,Amount,Description,Reference,Counterparty IBAN\n\
                   2026-10-01,-12.30,albert heijn,,\n\
                   2026-10-02,-12.30,Albert Heijn,,\n\
                   2026-10-02,-3.00,Coffee,bank-9,\n\
                   2026-10-03,-50,To ABN,,NL43 ABNA 0136 0063 96\n\
                   2026-10-04,20,Tikkie,,\n";
        // Several Revolut accounts: auto-detection must refuse.
        assert!(preview(store.connection(), &targets, csv, "revolut.csv", "auto").is_err());
        // The file names another bank.
        assert!(preview(store.connection(), &targets, csv, "swedbank.csv", "76").is_err());
        let p = preview(store.connection(), &targets, csv, "revolut.csv", "76").expect("preview");
        assert_eq!(p.account_name, "Revolut");
        assert_eq!(
            p.rows.iter().map(|r| r.status).collect::<Vec<_>>(),
            vec![
                CsvRowStatus::Duplicate,
                CsvRowStatus::New,
                CsvRowStatus::New,
                CsvRowStatus::New,
                CsvRowStatus::New
            ]
        );
        assert!(p.committable);

        {
            let tx = store.connection_mut().transaction().expect("tx");
            let w = Writer {
                tx,
                now: jiff::Zoned::now(),
            };
            let done = commit(
                &w,
                &targets,
                &CsvCommit {
                    preview_id: p.preview_id.clone(),
                    skip_flagged: false,
                },
            )
            .expect("commit");
            assert_eq!(done.imported, 4);
            w.tx.commit().expect("commit tx");
        }
        // The id is spent.
        assert!(begin(&p.preview_id).is_err());

        let conn = store.connection();
        let kinds: Vec<(String, i64, i64)> = conn
            .prepare("SELECT kind, source_account_id, destination_account_id FROM transactions WHERE id > 10 ORDER BY id")
            .expect("q")
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("rows")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");
        assert_eq!(kinds[0], ("withdrawal".into(), 76, 500));
        assert_eq!(kinds[2], ("transfer".into(), 76, 1));
        assert_eq!(kinds[3].0, "deposit");
        assert_eq!(kinds[3].2, 76);
        let postings: i64 = conn
            .query_row("SELECT count(*) FROM postings", [], |r| r.get(0))
            .expect("count");
        assert_eq!(postings, 10);
        let found: i64 = conn
            .query_row(
                "SELECT count(*) FROM transactions_search WHERE transactions_search MATCH 'Tikkie'",
                [],
                |r| r.get(0),
            )
            .expect("search");
        assert_eq!(found, 1);
        let run: String = conn
            .query_row(
                "SELECT stats FROM import_runs WHERE source = 'csv-import'",
                [],
                |r| r.get(0),
            )
            .expect("run");
        assert!(run.contains("\"imported\":4"));

        // Importing the same file again finds only duplicates.
        let again = preview(conn, &targets, csv, "revolut.csv", "76").expect("preview");
        assert_eq!(again.new_count, 0);
        assert_eq!(again.duplicate_count, 5);
    }

    #[test]
    fn flagged_rows_need_confirmation() {
        let mut store = ledger();
        let targets = accounts(&store);
        let csv =
            "Date,Amount,Description\n2026-10-01,-12.30,Other shop\nbad,1,x\n2026-10-05,-1,New\n";
        let p = preview(store.connection(), &targets, csv, "revolut.csv", "76").expect("preview");
        assert_eq!(p.possible_duplicate_count, 1);
        assert_eq!(p.rejected_count, 1);
        assert!(!p.committable);
        let tx = store.connection_mut().transaction().expect("tx");
        let w = Writer {
            tx,
            now: jiff::Zoned::now(),
        };
        let strict = CsvCommit {
            preview_id: p.preview_id.clone(),
            skip_flagged: false,
        };
        assert!(commit(&w, &targets, &strict).is_err());
        let skip = CsvCommit {
            skip_flagged: true,
            ..strict
        };
        assert_eq!(commit(&w, &targets, &skip).expect("commit").imported, 1);
    }

    #[test]
    fn rejects_wrong_currency() {
        let store = ledger();
        let targets = accounts(&store);
        let csv = "Date,Amount,Currency,Description\n2026-10-01,-1,USD,Shop\n";
        let error =
            preview(store.connection(), &targets, csv, "revolut.csv", "76").expect_err("currency");
        assert!(error.to_string().contains("currency"));
    }
}
