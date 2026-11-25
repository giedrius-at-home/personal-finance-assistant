//! The email's HTML, byte for byte as Python `spending_email_html.py` wrote
//! it: dashboard-styled tables with inline styles, every value escaped, and
//! charts drawn as table cells so nothing loads from the network.

use std::str::FromStr;

use finance_types::gateway::Direction;
use finance_types::report::outlook::DailySpendingForecast;
use jiff::civil::Date;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use super::{Risk, heaviest};
use crate::reporting::py_div;
use crate::trends::py_mul;

// Light design tokens (Python `design_tokens.py`).
const PAGE: &str = "#f4f3ef";
const CARD: &str = "#fbfaf7";
const CONTRAST: &str = "#292a2d";
const FG: &str = "#22221f";
const FG_MUTED: &str = "#686760";
const FG_ON_CONTRAST: &str = "#f4f3ef";
const BORDER: &str = "#dfded8";
const LINE: &str = "#dfded8";
const SOFT: &str = "#eeede8";
const CHART_1: &str = "#6288cf";
const CHART_2: &str = "#418345";
const CHART_4: &str = "#c3704f";
const CHART_FREE: &str = "#d9d7cf";

/// Python `html.escape` (quotes included).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}

/// `f"{value:,.2f}"` of a `Decimal`: half-even to cents, thousands commas.
fn grouped(value: Decimal) -> String {
    let mut cents = value.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
    cents.rescale(2);
    let text = cents.abs().to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
    let mut out = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    let sign = if cents.is_sign_negative() && !cents.is_zero() {
        "-"
    } else {
        ""
    };
    format!("{sign}{out}.{fraction}")
}

fn money(value: Decimal, currency: &str) -> String {
    escape(&format!(
        "{}{currency} {}",
        if value < Decimal::ZERO { "−" } else { "" },
        grouped(value.abs())
    ))
}

/// Python `max(Decimal(1), x)`.
fn at_least_one(x: Decimal) -> Decimal {
    if x > Decimal::ONE { x } else { Decimal::ONE }
}

/// Python `int(value / peak * 100)`.
fn percent(value: Decimal, peak: Decimal) -> i64 {
    py_mul(py_div(value, peak), Decimal::from(100))
        .trunc()
        .to_i64()
        .unwrap_or(0)
}

fn day(text: &str, format: &str) -> String {
    Date::from_str(text).map_or_else(|_| text.to_owned(), |d| d.strftime(format).to_string())
}

fn bar(label: &str, value: Decimal, maximum: Decimal, currency: &str, color: &str) -> String {
    let width = percent(value.abs(), at_least_one(maximum)).clamp(0, 100);
    format!(
        "<tr><td style=\"padding:10px 0 4px;font-size:13px\">{}</td><td align=\"right\" \
         style=\"font-size:14px;font-weight:bold\">{}</td></tr><tr><td colspan=\"2\">\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" \
         style=\"background:{SOFT};border-radius:5px\"><tr><td width=\"{width}%\" height=\"12\" \
         style=\"background:{color};border-radius:5px\"></td><td width=\"{}%\" height=\"12\">\
         </td></tr></table></td></tr>",
        escape(label),
        money(value, currency),
        100 - width
    )
}

fn card(
    label: &str,
    value: Decimal,
    final_balance: Option<Decimal>,
    currency: &str,
    color: &str,
    note: Option<String>,
) -> String {
    let ending = note.unwrap_or_else(|| match final_balance {
        Some(f) => format!("Final balance {}", money(f, currency)),
        None => "New savings over three months".to_owned(),
    });
    format!(
        "<td width=\"50%\" valign=\"top\" style=\"padding:17px;border-top:3px solid {color};\
         background:{CARD}\"><div style=\"font-size:12px;color:{FG_MUTED}\">{}</div>\
         <div style=\"font-size:25px;line-height:1.35;font-weight:bold;margin:8px 0\">{}</div>\
         <div style=\"font-size:12px;color:{FG_MUTED}\">{ending}</div></td>",
        escape(label),
        money(value, currency)
    )
}

fn stacked(
    label: &str,
    habit: Decimal,
    planned: Decimal,
    maximum: Decimal,
    currency: &str,
) -> String {
    let peak = at_least_one(maximum);
    let first = percent(habit, peak).clamp(0, 100);
    let second = percent(planned, peak).clamp(0, 100 - first);
    let mut cells = String::new();
    if first != 0 {
        cells.push_str(&format!(
            "<td width=\"{first}%\" height=\"12\" style=\"background:{CHART_1}\"></td>"
        ));
    }
    if second != 0 {
        cells.push_str(&format!(
            "<td width=\"{second}%\" height=\"12\" style=\"background:{CHART_4}\"></td>"
        ));
    }
    format!(
        "<tr><td style=\"padding:9px 0 4px;font-size:13px\">{}</td><td align=\"right\" \
         style=\"font-size:13px\"><b>{}</b></td></tr><tr><td colspan=\"2\">\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" \
         style=\"background:{SOFT};border-radius:5px\"><tr>{cells}<td width=\"{}%\" \
         height=\"12\"></td></tr></table></td></tr>",
        escape(label),
        money(habit + planned, currency),
        100 - first - second
    )
}

fn forecast_section(f: &DailySpendingForecast) -> String {
    let code = f.currency.code.as_str();
    let week = &f.days[..f.days.len().min(7)];
    // `max(..., default=Decimal(1))`: the first of the largest sums.
    let peak = week
        .iter()
        .map(|d| d.habit_expected + d.planned)
        .fold(None, |best: Option<Decimal>, x| match best {
            Some(b) if x <= b => Some(b),
            _ => Some(x),
        })
        .unwrap_or(Decimal::ONE);
    let bars: String = week
        .iter()
        .map(|d| {
            stacked(
                &day(&d.day, "%a %d %b"),
                d.habit_expected,
                d.planned,
                peak,
                code,
            )
        })
        .collect();
    let planned: String = f
        .upcoming
        .iter()
        .take(6)
        .map(|flow| {
            format!(
                "<tr><td style=\"padding:7px 0;border-bottom:1px solid {LINE}\">{} · {}</td>\
                 <td align=\"right\" style=\"padding:7px 0;border-bottom:1px solid {LINE}\">\
                 {}{}</td></tr>",
                day(&flow.day, "%d %b"),
                escape(&flow.name),
                if flow.direction == Direction::Inflow {
                    "+"
                } else {
                    ""
                },
                money(flow.amount, code)
            )
        })
        .collect();
    let habit_note = heaviest(f).map_or_else(String::new, |h| {
        format!(
            "Your heaviest habit day is <b>{}</b> (about {}). ",
            escape(&h.weekday),
            money(h.average, code)
        )
    });
    let coming_up = if planned.is_empty() {
        String::new()
    } else {
        format!(
            "<h3 style=\"font-size:15px;margin:20px 0 6px\">Coming up</h3>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" \
             style=\"font-size:13px\">{planned}</table>"
        )
    };
    let backtest = if f.backtest_windows > 0 {
        format!(", checked against {} past weeks", f.backtest_windows)
    } else {
        String::new()
    };
    format!(
        "<tr><td style=\"padding:25px 28px;border-bottom:1px solid {BORDER}\">\
         <div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED}\">THE FORECAST</div>\
         <h2 style=\"font-size:23px;margin:9px 0\">Your next seven days. {}</h2>\
         <p style=\"font-size:12px;color:{FG_MUTED}\"><span style=\"color:{CHART_1}\">■</span> habits \
         <span style=\"color:{CHART_4}\">■</span> planned payments and bills</p>\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\">{bars}</table>\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"4\" cellpadding=\"0\" \
         style=\"margin-top:18px\"><tr>{}{}</tr></table>{coming_up}\
         <p style=\"font-size:13px;line-height:1.7;margin:16px 0 0\">{habit_note}Income still \
         expected this month: <b>{}</b>.</p>\
         <p style=\"font-size:11px;line-height:1.6;color:{FG_MUTED};margin-top:12px\">\
         Forecast by {} from {} days of history{backtest}. Ranges are deliberately wide: they \
         add up each day's likely range.</p></td></tr>",
        escape(code),
        card(
            "Next 7 days: habits",
            f.next_week_habit,
            None,
            code,
            CHART_1,
            Some(format!(
                "Likely {}–{} · plus {} planned",
                money(f.next_week_lower, code),
                money(f.next_week_upper, code),
                money(f.next_week_planned, code)
            )),
        ),
        card(
            "Month end: habits",
            f.month_end_habit,
            None,
            code,
            CHART_4,
            Some(format!(
                "Likely {}–{} · {} still planned",
                money(f.month_end_lower, code),
                money(f.month_end_upper, code),
                money(f.month_remaining_planned, code)
            )),
        ),
        money(f.month_remaining_income, code),
        escape(&f.model),
        f.training_days,
    )
}

/// "What pushed today over": the day against its allowance, then the
/// purchases that could have been skipped and what to do instead.
fn day_section(day: &finance_types::notify::DaySummary) -> String {
    let totals: String = day
        .totals
        .iter()
        .filter(|t| t.over_by > Decimal::ZERO)
        .map(|t| {
            let code = t.currency.code.as_str();
            format!(
                "<p style=\"font-size:13px;line-height:1.7;margin:0 0 10px\">Spent today: <b>{}</b> \
                 · the plan allows <b>{}</b> a day · <b>{} over</b></p>",
                money(t.spent, code),
                money(t.allowance, code),
                money(t.over_by, code)
            )
        })
        .collect();
    let rows: String = day
        .avoidable
        .iter()
        .map(|p| {
            format!(
                "<tr><td style=\"padding:10px 0;border-bottom:1px solid {LINE}\">{}{}<br>\
                 <span style=\"font-size:11px;color:{FG_MUTED}\">Instead: {}</span></td>\
                 <td align=\"right\" style=\"padding:10px 0;border-bottom:1px solid {LINE}\"><b>{}</b></td></tr>",
                escape(&p.merchant),
                p.time
                    .as_ref()
                    .map(|t| format!(" <span style=\"color:{FG_MUTED}\">{}</span>", escape(t)))
                    .unwrap_or_default(),
                escape(&p.instead),
                money(p.amount, &p.currency.code)
            )
        })
        .collect();
    let skipped = if rows.is_empty() {
        String::new()
    } else {
        format!(
            "<div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED};margin-top:8px\">COULD HAVE BEEN AVOIDED</div>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" style=\"font-size:13px\">{rows}</table>"
        )
    };
    format!(
        "<tr><td style=\"padding:25px 28px;border-bottom:1px solid {BORDER}\">\
         <div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED}\">TODAY</div>\
         <h2 style=\"font-size:23px;margin:9px 0\">What pushed today over.</h2>{totals}{skipped}</td></tr>"
    )
}

fn absolute_max(values: &[Decimal]) -> Decimal {
    values
        .iter()
        .map(|v| v.abs())
        .fold(None, |best: Option<Decimal>, x| match best {
            Some(b) if x <= b => Some(b),
            _ => Some(x),
        })
        .unwrap_or(Decimal::ONE)
}

/// Python `render_spending_email`.
pub fn render(risk: &Risk, intro: &str, sample: bool, digest: bool) -> String {
    let mut sections = Vec::new();
    if let Some(today) = risk.day.as_ref().filter(|d| d.over()) {
        sections.push(day_section(today));
    }
    for window in risk.windows.iter().take(3) {
        let s = &window.signal;
        let code = window.currency.code.as_str();
        let maximum = absolute_max(&[s.spending, s.previous_spending, Decimal::ONE]);
        let mut bars = bar(
            "Previous three days",
            s.previous_spending,
            maximum,
            code,
            CHART_FREE,
        );
        bars.push_str(&bar(
            "Latest three complete days",
            s.spending,
            maximum,
            code,
            CHART_1,
        ));
        let savings_peak = absolute_max(&[
            window.planned_savings,
            window.expected_savings,
            Decimal::ONE,
        ]);
        let mut savings_bars = bar(
            "Following your plan",
            window.planned_savings,
            savings_peak,
            code,
            CHART_2,
        );
        savings_bars.push_str(&bar(
            "Continuing current habits",
            window.expected_savings,
            savings_peak,
            code,
            CHART_4,
        ));
        sections.push(format!(
            "<tr><td style=\"padding:25px 28px;border-bottom:1px solid {BORDER}\">\
             <div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED}\">THE SPENDING SPEED CHECK</div>\
             <h2 style=\"font-size:23px;margin:9px 0\">The receipts are in. {}</h2>\
             <p style=\"font-size:12px;color:{FG_MUTED}\">{}–{} · compared with {}–{}</p>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\">{bars}</table>\
             <p style=\"font-size:13px;line-height:1.7;margin:18px 0 0\">Unplanned spending: <b>{}</b><br>\
             Planned allowance for the window: <b>{}</b></p></td></tr>\
             <tr><td style=\"padding:25px 28px;border-bottom:1px solid {BORDER}\">\
             <div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED}\">THE NEXT THREE MONTHS</div>\
             <h2 style=\"font-size:23px;margin:9px 0 18px\">Two possible plotlines.</h2>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"4\" cellpadding=\"0\"><tr>{}{}</tr></table>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\">{savings_bars}</table>\
             <p style=\"font-size:11px;line-height:1.6;color:{FG_MUTED};margin-top:14px\">Estimates, not \
             promises. Negative savings mean drawing down your balance. Bars compare magnitudes; the \
             amounts show the sign.</p></td></tr>",
            escape(code),
            day(&s.start, "%d %b"),
            day(&s.end, "%d %b %Y"),
            day(&s.previous_start, "%d %b"),
            day(&s.previous_end, "%d %b"),
            money(s.unplanned, code),
            money(s.allowance, code),
            card("Following your plan", window.planned_savings, window.planned_final, code, CHART_2, None),
            card("Current habits", window.expected_savings, window.expected_final, code, CHART_4, None),
        ));
    }
    if !risk.groups.is_empty() {
        let rows: String = risk
            .groups
            .iter()
            .take(4)
            .map(|g| {
                format!(
                    "<tr><td style=\"padding:10px 0;border-bottom:1px solid {LINE}\">{}</td>\
                     <td align=\"right\" style=\"padding:10px 0;border-bottom:1px solid {LINE}\">\
                     <b>{}</b><br><span style=\"font-size:11px;color:{FG_MUTED}\">vs {} budget</span>\
                     </td></tr>",
                    escape(&g.group),
                    money(g.expected, &g.currency.code),
                    money(g.limit, &g.currency.code)
                )
            })
            .collect();
        sections.push(format!(
            "<tr><td style=\"padding:25px 28px;border-bottom:1px solid {BORDER}\">\
             <h2 style=\"font-size:22px;margin:0 0 10px\">Categories in the spotlight</h2>\
             <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" style=\"font-size:13px\">{rows}</table></td></tr>"
        ));
    }
    let notices: Vec<String> = risk
        .salary_at_risk
        .iter()
        .map(|(c, v)| format!("Next month's reserved salary at risk: {}.", money(*v, c)))
        .chain(
            risk.next_month_shortfall
                .iter()
                .map(|(c, v)| format!("Next month funding shortfall: {}.", money(*v, c))),
        )
        .collect();
    if !notices.is_empty() {
        sections.push(format!(
            "<tr><td style=\"padding:20px 28px;font-size:13px;line-height:1.8\">{}</td></tr>",
            notices.join("<br>")
        ));
    }
    sections.extend(risk.forecasts.iter().take(3).map(forecast_section));
    let edition = if sample {
        "PREVIEW EDITION · THIS IS A TEST"
    } else if digest {
        "YOUR DAILY FORECAST"
    } else {
        "YOUR PERSONAL MONEY BRIEFING"
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\"><title>The Rafael Report</title></head>\
         <body style=\"margin:0;background:{SOFT};color:{FG};font-family:Arial,Helvetica,sans-serif\">\
         <div style=\"display:none;max-height:0;overflow:hidden\">Your spending, your plan, and a quick move for future you.</div>\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\"><tr><td align=\"center\" style=\"padding:24px 8px\">\
         <table role=\"presentation\" width=\"100%\" cellspacing=\"0\" cellpadding=\"0\" style=\"max-width:640px;background:{CARD};border:1px solid {BORDER};border-radius:16px;overflow:hidden\">\
         <tr><td style=\"padding:20px 28px;border-bottom:1px solid {BORDER};color:{FG_MUTED};font-size:11px;letter-spacing:2px\">GIEDRIUS / FINANCE <span style=\"float:right\">RAFAEL</span></td></tr>\
         <tr><td style=\"padding:32px 28px;background:{CONTRAST};color:{CARD}\">\
         <div style=\"font-size:10px;letter-spacing:2px;color:{FG_ON_CONTRAST}\">{edition}</div>\
         <h1 style=\"font-size:42px;line-height:1.05;letter-spacing:-1.5px;margin:16px 0\">The Rafael<br>Report.</h1>\
         <p style=\"font-size:18px;line-height:1.5;margin:0;color:{FG_ON_CONTRAST}\">Your money. Front-page energy. 👀</p></td></tr>\
         <tr><td style=\"padding:24px 28px;font-size:16px;line-height:1.7\">{}</td></tr>{}\
         <tr><td style=\"padding:28px\"><div style=\"font-size:10px;letter-spacing:2px;color:{FG_MUTED}\">YOUR NEXT MOVE</div>\
         <h2 style=\"font-size:23px;margin:10px 0\">Protect the future-you fund.</h2>\
         <p style=\"font-size:14px;line-height:1.7\">Check what is left in your plan before the next optional purchase. \
         If a friend paid you back, assign it to the right category so the numbers tell the full story.</p>\
         <a href=\"{}\" style=\"display:inline-block;\
         padding:14px 22px;background:{CONTRAST};color:{FG_ON_CONTRAST};text-decoration:none;border-radius:8px;font-weight:bold;font-size:14px\">Open my dashboard ↗</a>\
         <p style=\"font-size:14px;margin-top:25px\">You have got this.<br><b>— Rafael</b></p></td></tr>\
         <tr><td style=\"padding:18px 28px;background:{PAGE};font-size:11px;color:{FG_MUTED};line-height:1.6\">\
         Built from your recorded spending and entered plans. Refunds reduce net spending; movements between your own accounts are excluded. \
         Expected income is counted once. A small reset today can make room for what matters tomorrow.</td></tr>\
         </table></td></tr></table></body></html>",
        escape(intro),
        sections.concat(),
        super::PLAN_LINK,
    )
}
