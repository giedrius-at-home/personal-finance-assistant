//! Number and date formatting (en-IE style: €1,234.56, 26 Oct 2026).

use rust_decimal::{Decimal, RoundingStrategy};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec",
];
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn symbol(currency: &str) -> Option<&'static str> {
    match currency {
        "EUR" => Some("€"),
        "USD" => Some("$"),
        "GBP" => Some("£"),
        "CHF" => Some("CHF\u{a0}"),
        _ => None,
    }
}

fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `€1,234.56`, `−€12.00`; `decimals` 0 for compact tiles.
pub fn money_dp(amount: Decimal, currency: &str, decimals: u32) -> String {
    let rounded = amount.round_dp_with_strategy(decimals, RoundingStrategy::MidpointAwayFromZero);
    let negative = rounded.is_sign_negative() && !rounded.is_zero();
    let text = format!("{:.*}", decimals as usize, rounded.abs());
    let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
    let number = if frac.is_empty() {
        group(whole)
    } else {
        format!("{}.{frac}", group(whole))
    };
    let body = match symbol(currency) {
        Some(sym) => format!("{sym}{number}"),
        None => format!("{number}\u{a0}{currency}"),
    };
    if negative {
        format!("\u{2212}{body}")
    } else {
        body
    }
}

fn parts(date: &str) -> Option<(&str, usize, &str)> {
    let year = date.get(..4)?;
    let month: usize = date.get(5..7)?.parse().ok()?;
    let day = date.get(8..10).unwrap_or("");
    (1..=12).contains(&month).then_some((year, month, day))
}

/// `2026-10-26` → `26 Oct 2026`.
pub fn date(date: &str) -> String {
    match parts(date) {
        Some((y, m, d)) => format!("{} {} {y}", d.trim_start_matches('0'), MONTHS[m - 1]),
        None => date.to_owned(),
    }
}

/// `2026-10` → `October 2026`.
pub fn month_long(month: &str) -> String {
    match parts(month) {
        Some((y, m, _)) => format!("{} {y}", MONTHS_LONG[m - 1]),
        None => month.to_owned(),
    }
}

/// `2026-10` → `Oct`.
pub fn month_short(month: &str) -> String {
    match parts(month) {
        Some((_, m, _)) => MONTHS[m - 1].to_owned(),
        None => month.to_owned(),
    }
}

pub fn count(n: u64) -> String {
    group(&n.to_string())
}

/// Shift a `YYYY-MM` month by `delta` months.
pub fn add_months(month: &str, delta: i32) -> String {
    let Some((y, m, _)) = parts(month) else {
        return month.to_owned();
    };
    let index = y.parse::<i32>().unwrap_or(1970) * 12 + m as i32 - 1 + delta;
    format!(
        "{:04}-{:02}",
        index.div_euclid(12),
        index.rem_euclid(12) + 1
    )
}

/// Last day of a `YYYY-MM` month as `YYYY-MM-DD`.
pub fn month_end(month: &str) -> String {
    let Some((y, m, _)) = parts(month) else {
        return format!("{month}-31");
    };
    let year: i32 = y.parse().unwrap_or(1970);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    format!("{month}-{days:02}")
}

/// `2026-10-26T21:07:00+01:00` or `2026-10-26 21:07` → `26 Oct, 21:07`.
pub fn stamp(text: &str) -> String {
    let day = match parts(text) {
        Some((_, m, d)) => format!("{d} {}", MONTHS[m - 1]),
        None => return text.to_owned(),
    };
    match text.get(11..16) {
        Some(time) => format!("{day}, {time}"),
        None => day,
    }
}

/// `2026-10-01` → `Oct 26` (en-GB short month, two-digit year).
pub fn month_short_year(date: &str) -> String {
    match parts(date) {
        Some((y, m, _)) => format!("{} {}", MONTHS[m - 1], &y[2..]),
        None => date.to_owned(),
    }
}

/// `2026-10-05` → `5 Oct` (en-GB day and short month).
pub fn day_month(date: &str) -> String {
    match parts(date) {
        Some((_, m, d)) => format!("{} {}", d.trim_start_matches('0'), MONTHS[m - 1]),
        None => date.to_owned(),
    }
}

/// Days in the month of `YYYY-MM-…`.
pub fn days_in_month(date: &str) -> u32 {
    month_end(&date[..7.min(date.len())])
        .get(8..10)
        .and_then(|d| d.parse().ok())
        .unwrap_or(30)
}
