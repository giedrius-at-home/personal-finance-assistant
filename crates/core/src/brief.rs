//! Gisel's daily brief: once a day at the owner's hour, what is left in
//! the plan, what is close to its limit, which bills are coming, and how the
//! habits moved (the last seven days against the four weeks before). Mistral
//! turns the facts into a short spoken script with one thing to change and a
//! thought woven in; Voxtral (Esmeralda) speaks it; the phone gets a push that opens the
//! player.
//!
//! The model only words what is here: every number it says must appear in
//! the facts, or the fixed script stands in. Writing and speaking are the
//! [`BriefWriter`] and [`Speaker`] the app passes in (`finance-notify`).

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use finance_types::notify::{
    Brief, BriefBill, BriefFacts, BriefGroup, BriefHabit, BriefLeft, BriefLength, BriefSettings,
    EMOTION_TAGS, EXPRESSIVENESS_RANGE, PACE_TAGS, SOUND_TAGS, VoiceOption, known_tag,
    mood_versions, plain_script,
};
use finance_types::report::Currency;
use finance_types::report::allocation::GroupAllocation;
use finance_types::report::subscriptions::ScheduledBillPayment;
use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::Deserialize;

use crate::groups::is_recurring_debit;
use crate::load::Snapshot;
use crate::model::Kind;
use crate::pyfmt::isoformat_utc;
use crate::service::Service;
use crate::spending_email::{claim, complete, latest_due};
use crate::{Ctx, Error, Result, budget, plans};

/// The scheduler job (`evaluation_schedule`).
pub const JOB: &str = "voice_brief";
/// Briefs older than this many days are deleted.
const KEEP_DAYS: i64 = 14;
/// A group counts as near its limit from this share used.
const NEAR: i64 = 80;
/// A week this much above or below the usual is a change.
const CHANGE: i64 = 15;
/// Weeks of history the usual week is the average of.
const BASELINE_WEEKS: usize = 4;
/// Habits named, at most.
const HABITS: usize = 3;
/// Weeks under this (in the currency's units) are too small to talk about.
const SMALL: i64 = 10;

/// The persona and rules for the daily script; `{name}`, `{owner}`,
/// `{greeting}`, `{tone}`, `{lo}` and `{hi}` are filled from the settings
/// and the day.
pub const BRIEF_SYSTEM: &str = "You are {name}, from Spain, and you talk to {owner} every \
morning in a short voice message about their money. {owner} is an adult who asked for it raw: you \
are loud, opinionated and personal, never bland or neutral, never a polite assistant. {tone} Talk \
to {owner} directly and use the name once or twice. Only ever roast the money choices in the \
facts, never looks, body, health, family or anything else. You speak English with a few natural \
Spanish words, at most five per script, such as vale, venga, hombre, tranqui, ojo, madre mía, \
joder, hostia, pues nada, bueno, and \"ja ja\" for a laugh; never whole Spanish sentences. Mostly \
correct grammar, with a light Spanish flavour now and then (\"is no good\"); never misspell words \
to fake an accent. Write for the ear: vary the sentence length, let natural pauses come from \
commas and \"…\", ask a rhetorical question now and then, and use an exclamation when something \
is really good or really bad. Your voice is read by a speech model that takes delivery tags in \
square brackets, which it acts on and never says aloud: [emotion=X] sets the feeling from there \
on, X one of {emotions}; [pace=slow] or [pace=fast] changes speed for a moment ({paces}); and \
the sounds {sounds}. Use these tags and no other brackets: start with an emotion tag, change the \
emotion where the mood of the facts changes (sarcastic, disappointed or angry for an overspend, \
excited or proud for a habit that improves, a sigh before bad news, a laugh at something silly), \
two to six tags in all, never two in a row of the same kind, no stage directions in words. You \
get today's facts as JSON. Return JSON with \"title\" (a push notification title, under 60 \
characters, in the same voice), \"script\" (what you say out loud, {lo} to {hi} words, no lists, \
no emoji, no markdown) and \"gif\" (two to five plain English words to search GIPHY for the one \
reaction GIF that sums up today, in today's mood: \"slow clap\", \"money on fire\", \"disappointed \
face palm\", \"you go girl\", \"absolutely not\"; no names, no numbers). The script: start with \
\"{greeting}\"; say what is left and per day; mention the group closest to or over its limit and \
any bill due soon; say plainly whether the habits are changing or not (use direction and \
weeks_in_a_row); tell {owner} exactly one concrete thing to do or not do today, as an order. \
Weave one wise, slightly twisted thought into the flow, tied to today's facts, the way a friend \
says it in passing: never label it, never call it a lesson, never tack it on as a moral at the \
end. Use only numbers that appear in the facts, written exactly as given (\"€412\", \"90 \
percent\" for used_percent 90). Never invent purchases, merchants, amounts or dates. No \
investment advice.";

/// How the persona treats the owner today; the days take turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Teasing like an older sibling: a little insulting and bossy, then a
    /// real push.
    MeanButLoving,
    /// No softening: brutal on a bad day, loud hype on a good one.
    Savage,
}

impl Tone {
    /// The day's tone: even and odd days of the year take turns.
    pub fn of_day(date: &str) -> Self {
        let day = date.parse::<Date>().map_or(0, |d| d.day_of_year());
        if day % 2 == 0 {
            Self::Savage
        } else {
            Self::MeanButLoving
        }
    }

    fn rules(self) -> &'static str {
        match self {
            Self::MeanButLoving => {
                "Today you are mean but loving, like an older sibling: tease {owner}, call them \
names (\"you absolute disaster\", \"cabrón\"), boss them around and forbid things outright (\"no \
takeaway for you today, I mean it\"), swear a little; then always land a real push and cheer \
anything that went well, warmly and loudly."
            }
            Self::Savage => {
                "Today you are full savage: on anything bad in the facts, roast {owner} hard, \
insult the choices, swear freely (fuck, shit, joder, coño), mock the habit and take things away \
from them, no softening and no apology. On anything good, hype {owner} up just as hard, swearing \
in celebration. Exaggerate for the laugh, but every fact stays exact."
            }
        }
    }
}

/// The owner's own words on the persona's style (Manage → Settings), added
/// to the prompt: at most this many characters, no digits.
pub const MAX_STYLE: usize = 300;

/// The longest name the persona calls the owner.
pub const MAX_OWNER_NAME: usize = 40;

/// The script's first words: `Hola Giedrius, Gisel here.`, or without a
/// name `Hola, Gisel here.`
pub fn greeting(settings: &BriefSettings) -> String {
    let name = settings.persona.name();
    match settings.owner_name.trim() {
        "" => format!("Hola, {name} here."),
        owner => format!("Hola {owner}, {name} here."),
    }
}

/// The system prompt for `settings` on the day of `facts`: persona, tone,
/// length and style notes.
pub fn system_prompt(settings: &BriefSettings, facts: &BriefFacts) -> String {
    let (lo, hi) = settings.length.words();
    let owner = match settings.owner_name.trim() {
        "" => "the owner",
        name => name,
    };
    let prompt = BRIEF_SYSTEM
        .replace("{tone}", Tone::of_day(&facts.date).rules())
        .replace("{emotions}", &EMOTION_TAGS.join(", "))
        .replace("{paces}", &PACE_TAGS.join(", "))
        .replace(
            "{sounds}",
            &SOUND_TAGS
                .iter()
                .map(|t| format!("[{t}]"))
                .collect::<Vec<_>>()
                .join(", "),
        )
        .replace("{greeting}", &greeting(settings))
        .replace("{owner}", owner)
        .replace("{name}", settings.persona.name())
        .replace("{lo}", &lo.to_string())
        .replace("{hi}", &hi.to_string());
    let style = settings.style.trim();
    if style.is_empty() {
        prompt
    } else {
        format!(
            "{prompt} The owner's notes on your style (follow them unless they break the rules above): {style}"
        )
    }
}

/// What the writer gives back: the push title, the script with its
/// delivery tags, and the GIPHY search for the day's GIF (may be empty).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub title: String,
    pub script: String,
    pub gif: String,
}

/// Writes the title, script and GIF search from the facts, as `settings`
/// say.
pub trait BriefWriter {
    fn write(
        &self,
        facts: &BriefFacts,
        settings: &BriefSettings,
    ) -> std::result::Result<Written, String>;
}

/// Turns the script into audio (MP3) in `voice` (an id or slug), at
/// `expressiveness` (1.0 calm to 5.0 most expressive). The script may
/// carry Voxtral's delivery tags (`[emotion=warm]`, `[sigh]`).
pub trait Speaker {
    fn speak(
        &self,
        script: &str,
        voice: &str,
        expressiveness: f64,
    ) -> std::result::Result<Vec<u8>, String>;
}

/// No voice: the brief goes out as text.
#[derive(Debug)]
pub struct Silent;

impl Speaker for Silent {
    fn speak(&self, _: &str, _: &str, _: f64) -> std::result::Result<Vec<u8>, String> {
        Err("Voice is off.".into())
    }
}

/// The moods that fit the day, best first: low when something is over its
/// limit or the avoidable habit grows, bright when it shrinks (more so after
/// three weeks), confident otherwise. Neutral is always the last resort.
pub fn moods(facts: &BriefFacts) -> &'static [&'static str] {
    let habit = facts.avoidable.as_ref();
    let direction = habit.map_or("steady", |h| h.direction.as_str());
    if !facts.over_limit.is_empty() || direction == "up" {
        &["frustrated", "sad", "serious", "neutral"]
    } else if direction == "down" && habit.is_some_and(|h| h.weeks_in_a_row >= 3) {
        &["excited", "happy", "cheerful", "confident", "neutral"]
    } else if direction == "down" {
        &["happy", "cheerful", "excited", "confident", "neutral"]
    } else {
        &["confident", "calm", "neutral"]
    }
}

/// The voice to speak in: with `match_mood`, the chosen voice's version
/// that best fits the day; else (or without versions) the chosen voice.
pub fn pick_voice(settings: &BriefSettings, voices: &[VoiceOption], facts: &BriefFacts) -> String {
    let chosen = settings.chosen_voice().to_owned();
    if !settings.match_mood {
        return chosen;
    }
    let versions = mood_versions(&chosen, voices);
    moods(facts)
        .iter()
        .find_map(|want| versions.iter().find(|(_, m)| m == want))
        .map_or(chosen, |(v, _)| v.id.clone())
}

/// Whole units, as said out loud: "€412", "1,250 SEK".
pub fn say_money(amount: Decimal, currency: &Currency) -> String {
    let units = amount.round_dp(0).to_i64().unwrap_or_default();
    let digits = units.abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if units < 0 { "-" } else { "" };
    match currency.code.as_str() {
        "EUR" => format!("{sign}€{grouped}"),
        "USD" => format!("{sign}${grouped}"),
        "GBP" => format!("{sign}£{grouped}"),
        code => format!("{sign}{grouped} {code}"),
    }
}

fn percent(part: Decimal, whole: Decimal) -> i64 {
    if whole <= Decimal::ZERO {
        return 0;
    }
    (part / whole * Decimal::from(100))
        .round_dp(0)
        .to_i64()
        .unwrap_or_default()
}

/// How the last week compares with the weeks before it. `weeks` is newest
/// first: `weeks[0]` the last seven days, then the four weeks before.
pub fn habit(group: &str, weeks: &[Decimal], currency: &Currency) -> Option<BriefHabit> {
    let (last, before) = weeks.split_first()?;
    let before: Vec<Decimal> = before.iter().take(BASELINE_WEEKS).copied().collect();
    if before.is_empty() {
        return None;
    }
    let usual = before.iter().sum::<Decimal>() / Decimal::from(before.len());
    if *last < Decimal::from(SMALL) && usual < Decimal::from(SMALL) {
        return None;
    }
    let change = if usual > Decimal::ZERO {
        percent(*last - usual, usual)
    } else {
        100
    };
    let direction = if change >= CHANGE {
        "up"
    } else if change <= -CHANGE {
        "down"
    } else {
        "steady"
    };
    // How many weeks in a row it moved this way, newest first.
    let mut streak = 1;
    if direction != "steady" {
        for pair in weeks.windows(2).skip(1) {
            let moved = match direction {
                "up" => pair[0] > pair[1],
                _ => pair[0] < pair[1],
            };
            if !moved {
                break;
            }
            streak += 1;
        }
    } else {
        for w in &before {
            if percent(*w - usual, usual).abs() >= CHANGE {
                break;
            }
            streak += 1;
        }
    }
    Some(BriefHabit {
        group: group.to_owned(),
        last_week: say_money(*last, currency),
        usual_week: say_money(usual, currency),
        change_percent: change,
        direction: direction.into(),
        weeks_in_a_row: streak,
    })
}

fn weekday(date: Date) -> String {
    date.strftime("%A").to_string()
}

/// "today", "tomorrow" or the weekday of `due` (within a week of `today`).
fn due_word(today: Date, due: Date) -> String {
    match (due - today).get_days() {
        0 => "today".into(),
        1 => "tomorrow".into(),
        _ => weekday(due),
    }
}

/// Today's facts from this month's group `limits` (per currency), the
/// month's scheduled `bills` and the ledger.
pub fn facts(
    limits: &[(Currency, Vec<GroupAllocation>)],
    bills: &[ScheduledBillPayment],
    snap: &Snapshot,
    now: &Zoned,
    ticks: &HashMap<String, bool>,
) -> BriefFacts {
    let ctx = Ctx::new(snap, now);
    let today = ctx.today();
    let next_month = crate::period::shift_month(ctx.current_month(), 1);
    let days_left = i64::from((next_month - today).get_days()).max(1);
    let mut facts = BriefFacts {
        date: today.to_string(),
        weekday: weekday(today),
        ..BriefFacts::default()
    };

    let mut currencies: Vec<Currency> = Vec::new();
    for (currency, groups) in limits {
        currencies.push(currency.clone());
        let limit: Decimal = groups.iter().map(|g| g.limit).sum();
        if limit <= Decimal::ZERO {
            continue;
        }
        let left: Decimal = groups.iter().map(|g| g.remaining.max(Decimal::ZERO)).sum();
        facts.left.push(BriefLeft {
            left: say_money(left, currency),
            per_day: say_money(left / Decimal::from(days_left), currency),
            days_left,
        });
        let mut rows: Vec<(i64, BriefGroup)> = groups
            .iter()
            .filter(|g| g.limit > Decimal::ZERO)
            .map(|g| {
                let used = percent(g.spent, g.limit);
                (
                    used,
                    BriefGroup {
                        group: g.group.clone(),
                        used_percent: used,
                        left: say_money(g.remaining.max(Decimal::ZERO), currency),
                    },
                )
            })
            .collect();
        rows.sort_by_key(|(used, _)| std::cmp::Reverse(*used));
        for (used, g) in rows {
            if used > 100 {
                facts.over_limit.push(g);
            } else if used >= NEAR {
                facts.near_limit.push(g);
            }
        }
    }
    facts.over_limit.truncate(3);
    facts.near_limit.truncate(3);

    let mut soon: Vec<(Date, BriefBill)> = bills
        .iter()
        .filter(|p| p.paid_on.is_none())
        .filter_map(|p| {
            let due: Date = p.due_on.parse().ok()?;
            let days = (due - today).get_days();
            (0..=6).contains(&days).then(|| {
                (
                    due,
                    BriefBill {
                        name: p.name.chars().take(40).collect(),
                        due: due_word(today, due),
                        amount: say_money(p.amount.amount, &p.amount.currency),
                    },
                )
            })
        })
        .collect();
    soon.sort_by_key(|(d, _)| *d);
    facts.bills_soon = soon.into_iter().map(|(_, b)| b).take(3).collect();

    // Everyday spending by local day, group and currency.
    let group_of = |category: &str| {
        snap.group_mappings
            .iter()
            .filter(|m| m.category == category)
            .filter_map(|m| m.group.as_deref().filter(|g| !g.is_empty()))
            .next_back()
            .unwrap_or("Other")
            .to_owned()
    };
    let weeks = 1 + BASELINE_WEEKS;
    let first = today
        .checked_sub(jiff::Span::new().days(7 * weeks as i64))
        .unwrap_or(today);
    let mut by_group: BTreeMap<(String, String), Vec<Decimal>> = BTreeMap::new();
    let mut avoidable: BTreeMap<String, Vec<Decimal>> = BTreeMap::new();
    let mut yesterday: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut currency_of: HashMap<String, Currency> = HashMap::new();
    let spending = budget::spending_transactions(&ctx, &plans::planned_payment_views(&ctx));
    for t in &spending {
        let day = t.purchased_on.unwrap_or_else(|| t.occurred_at.date());
        if t.kind != Kind::Outflow
            || day >= today
            || day < first
            || t.subscription_id.is_some()
            || is_recurring_debit(t.description.as_deref())
        {
            continue;
        }
        let back = (today - day).get_days() - 1;
        let Ok(week) = usize::try_from(back / 7) else {
            continue;
        };
        if week >= weeks {
            continue;
        }
        let code = t.currency.code.clone();
        currency_of
            .entry(code.clone())
            .or_insert_with(|| t.currency.clone());
        let group = t
            .category
            .as_deref()
            .filter(|c| !c.is_empty())
            .map_or_else(|| "Other".to_owned(), group_of);
        if back == 0 {
            *yesterday.entry(code.clone()).or_default() += t.amount;
        }
        if crate::day_summary::is_avoidable(ticks, &group) {
            avoidable
                .entry(code.clone())
                .or_insert_with(|| vec![Decimal::ZERO; weeks])[week] += t.amount;
        }
        by_group
            .entry((code, group))
            .or_insert_with(|| vec![Decimal::ZERO; weeks])[week] += t.amount;
    }
    for (code, amount) in &yesterday {
        if let Some(c) = currency_of.get(code) {
            facts.spent_yesterday.push(say_money(*amount, c));
        }
    }
    let mut habits: Vec<BriefHabit> = by_group
        .iter()
        .filter(|((_, group), _)| group != "Other")
        .filter_map(|((code, group), w)| habit(group, w, currency_of.get(code)?))
        .filter(|h| h.direction != "steady")
        .collect();
    habits.sort_by_key(|h| std::cmp::Reverse(h.change_percent.abs()));
    habits.truncate(HABITS);
    facts.habits = habits;
    // The main currency's avoidable spending: the plan's first, else EUR.
    let main = currencies
        .first()
        .map_or_else(|| "EUR".to_owned(), |c| c.code.clone());
    facts.avoidable = avoidable.get(&main).and_then(|w| {
        habit(
            "avoidable spending (eating out, coffee, shopping and the like)",
            w,
            currency_of.get(&main)?,
        )
    });
    facts
}

/// A delivery tag, as `[emotion=warm]`; group 1 is what is inside.
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\]").expect("tag pattern"));

/// More tags than this and the delivery turns into a performance.
const MAX_TAGS: usize = 8;

static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+(?:[.,]\d+)*").expect("number pattern"));

/// The numbers the script may say: every number in the facts, and the
/// small counts any sentence may use ("one thing", "two weeks").
fn allowed_numbers(facts: &BriefFacts) -> Vec<String> {
    let text = serde_json::to_string(facts).unwrap_or_default();
    let mut numbers: Vec<String> = NUMBER
        .find_iter(&text)
        .map(|m| m.as_str().to_owned())
        .collect();
    numbers.extend((0..=7).map(|n| n.to_string()));
    numbers
}

/// The model's GIF search, cleaned: plain words only (letters, spaces,
/// apostrophes, hyphens), at most six words and 50 characters; anything
/// else is dropped rather than refusing the brief.
pub fn gif_query(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let query = words.join(" ");
    let plain = query
        .chars()
        .all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-');
    if plain && words.len() <= 6 && query.chars().count() <= 50 {
        query.to_lowercase()
    } else {
        String::new()
    }
}

/// Check the model's answer: a title and a script of the right size that
/// says no number the facts do not hold, and the GIF search.
pub fn parse_brief(
    text: &str,
    facts: &BriefFacts,
    length: BriefLength,
) -> std::result::Result<Written, String> {
    #[derive(Deserialize)]
    struct Reply {
        title: String,
        script: String,
        #[serde(default)]
        gif: String,
    }
    let invalid = |why: &str| format!("Mistral's brief was refused: {why}.");
    let reply: Reply = serde_json::from_str(text).map_err(|_| invalid("not the JSON asked for"))?;
    let (title, script) = (reply.title.trim(), reply.script.trim());
    if !(1..=80).contains(&title.chars().count()) || title.contains('[') {
        return Err(invalid("title length"));
    }
    let tags: Vec<&str> = TAG
        .captures_iter(script)
        .map(|c| c.get(1).map_or("", |m| m.as_str()))
        .collect();
    if tags.len() > MAX_TAGS || !tags.iter().all(|t| known_tag(t)) {
        return Err(invalid("delivery tags"));
    }
    let spoken = plain_script(script);
    if spoken.contains(['[', ']']) {
        return Err(invalid("formatting"));
    }
    let words = spoken.split_whitespace().count();
    let (lo, hi) = length.words();
    if !(lo * 2 / 3..=hi * 4 / 3).contains(&words) || script.chars().count() > hi * 10 {
        return Err(invalid("script length"));
    }
    if spoken.contains("http") || spoken.contains('*') || spoken.contains('#') {
        return Err(invalid("formatting"));
    }
    if spoken.to_lowercase().contains("life lesson") {
        return Err(invalid("a labelled life lesson"));
    }
    let allowed = allowed_numbers(facts);
    for n in NUMBER.find_iter(&format!("{title} {spoken}")) {
        if !allowed.iter().any(|a| a == n.as_str()) {
            return Err(invalid("a number not in the facts"));
        }
    }
    Ok(Written {
        title: title.to_owned(),
        script: script.to_owned(),
        gif: gif_query(&reply.gif),
    })
}

/// What the persona says when Mistral is not available.
pub fn fixed_brief(facts: &BriefFacts, settings: &BriefSettings) -> Written {
    let mut lines = vec![format!("[emotion=warm]{}", greeting(settings))];
    let title = match facts.left.first() {
        Some(l) => {
            lines.push(format!(
                "Vale, you have {} left for the next {} days, about {} a day.",
                l.left, l.days_left, l.per_day
            ));
            format!("{} left, {} a day", l.left, l.per_day)
        }
        None => "Your money, quickly".to_owned(),
    };
    if let Some(g) = facts.over_limit.first() {
        lines.push(format!(
            "[sigh] Ojo, {} is over its limit. Leave it alone for now.",
            g.group
        ));
    } else if let Some(g) = facts.near_limit.first() {
        lines.push(format!(
            "{} is at {} percent, {} left. Tranqui, but watch it.",
            g.group, g.used_percent, g.left
        ));
    }
    if let Some(b) = facts.bills_soon.first() {
        lines.push(format!("{} of {} goes out {}.", b.name, b.amount, b.due));
    }
    match &facts.avoidable {
        Some(h) if h.direction == "down" => lines.push(format!(
            "Your avoidable spending was {} this week, {} usually. It is changing, bueno. Funny how the small coffees turn into a whole month.",
            h.last_week, h.usual_week
        )),
        Some(h) if h.direction == "up" => lines.push(format!(
            "Avoidable spending: {} this week against {} usually. Is no good, the habit is not changing. Skip one thing today; tomorrow you will not miss it.",
            h.last_week, h.usual_week
        )),
        Some(h) => lines.push(format!(
            "Avoidable spending sits at about {} a week, same as always. Pues nada, same habits make the same month. Change one small thing today.",
            h.usual_week
        )),
        None => lines.push("Change one small thing today; that is how a month moves.".into()),
    }
    lines.push("Venga.".into());
    let direction = facts
        .avoidable
        .as_ref()
        .map_or("steady", |h| h.direction.as_str());
    let gif = if !facts.over_limit.is_empty() || direction == "up" {
        "disappointed face palm"
    } else if direction == "down" {
        "proud of you"
    } else {
        "lets go"
    };
    Written {
        title,
        script: lines.join(" "),
        gif: gif.into(),
    }
}

/// The day's title, script and GIF search, and who wrote them.
pub fn compose(
    facts: &BriefFacts,
    settings: &BriefSettings,
    writer: &dyn BriefWriter,
) -> (Written, &'static str) {
    match writer.write(facts, settings) {
        Ok(written) => (written, "mistral"),
        Err(e) => {
            eprintln!("brief: fixed script ({e})");
            (fixed_brief(facts, settings), "fixed")
        }
    }
}

// Storage ----------------------------------------------------------------------

pub fn load_settings(conn: &Connection) -> Result<BriefSettings> {
    let text: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM brief_settings WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .or_else(|e| match e {
            rusqlite::Error::SqliteFailure(..) => Ok(None),
            other => Err(other),
        })?;
    text.map_or_else(
        || Ok(BriefSettings::default()),
        |t| serde_json::from_str(&t).map_err(|e| Error::Invalid(format!("brief settings: {e}"))),
    )
}

pub fn validate(mut s: BriefSettings) -> Result<BriefSettings> {
    if s.hour > 23 {
        return Err(Error::Invalid("Choose an hour from 0 to 23.".into()));
    }
    if !EXPRESSIVENESS_RANGE.contains(&s.expressiveness) {
        return Err(Error::Invalid(
            "Expressiveness goes from 1.0 to 5.0.".into(),
        ));
    }
    s.style = s.style.trim().to_owned();
    if s.style.chars().count() > MAX_STYLE
        || s.style
            .chars()
            .any(|c| c.is_ascii_digit() || (c.is_control() && c != '\n'))
    {
        return Err(Error::Invalid(format!(
            "The style note: at most {MAX_STYLE} characters and no numbers."
        )));
    }
    s.voice_id = s.voice_id.trim().to_owned();
    if s.voice_id.len() > 128
        || s.voice_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Error::Invalid("The voice id is not valid.".into()));
    }
    s.owner_name = s
        .owner_name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if s.owner_name.chars().count() > MAX_OWNER_NAME
        || !s
            .owner_name
            .chars()
            .all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-' || c == '.')
    {
        return Err(Error::Invalid(format!(
            "Your name: at most {MAX_OWNER_NAME} characters, letters only."
        )));
    }
    s.telegram_chat = s.telegram_chat.trim().to_owned();
    let digits = s
        .telegram_chat
        .strip_prefix('-')
        .unwrap_or(&s.telegram_chat);
    if !s.telegram_chat.is_empty()
        && (digits.is_empty() || digits.len() > 20 || !digits.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::Invalid(
            "The Telegram chat id is a number, like 123456789.".into(),
        ));
    }
    Ok(s)
}

pub fn save_settings(conn: &Connection, s: &BriefSettings) -> Result<()> {
    let json = serde_json::to_string(s).map_err(|e| Error::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO brief_settings(singleton, payload_json) VALUES (1, ?1) \
         ON CONFLICT(singleton) DO UPDATE SET payload_json = excluded.payload_json",
        [json],
    )?;
    Ok(())
}

fn row_brief(r: &rusqlite::Row<'_>) -> rusqlite::Result<Brief> {
    let facts: String = r.get(5)?;
    Ok(Brief {
        date: r.get(0)?,
        created_at: r.get(1)?,
        title: r.get(2)?,
        script: r.get(3)?,
        source: r.get(4)?,
        facts: serde_json::from_str(&facts).unwrap_or_default(),
        has_audio: r.get(6)?,
        voice_id: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        persona: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        gif: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
    })
}

const SELECT: &str = "SELECT brief_date, created_at, title, script, source, facts_json, \
                      audio IS NOT NULL, voice_id, persona, gif FROM voice_briefs";

pub fn latest(conn: &Connection) -> Result<Option<Brief>> {
    Ok(conn
        .query_row(
            &format!("{SELECT} ORDER BY brief_date DESC LIMIT 1"),
            [],
            row_brief,
        )
        .optional()?)
}

pub fn audio(conn: &Connection, date: &str) -> Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT audio FROM voice_briefs WHERE brief_date = ?1",
            [date],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

pub fn save(conn: &Connection, brief: &Brief, audio: Option<&[u8]>) -> Result<()> {
    let facts = serde_json::to_string(&brief.facts).map_err(|e| Error::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO voice_briefs (brief_date, created_at, title, script, source, facts_json, audio, voice_id, persona, gif)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(brief_date) DO UPDATE SET created_at = ?2, title = ?3, script = ?4,
           source = ?5, facts_json = ?6, audio = ?7, voice_id = ?8, persona = ?9, gif = ?10",
        params![
            brief.date,
            brief.created_at,
            brief.title,
            brief.script,
            brief.source,
            facts,
            audio,
            brief.voice_id,
            brief.persona,
            brief.gif
        ],
    )?;
    if let Ok(date) = brief.date.parse::<Date>()
        && let Ok(oldest) = date.checked_sub(jiff::Span::new().days(KEEP_DAYS))
    {
        conn.execute(
            "DELETE FROM voice_briefs WHERE brief_date < ?1",
            [oldest.to_string()],
        )?;
    }
    Ok(())
}

impl Service {
    pub fn brief_settings(&self) -> std::result::Result<BriefSettings, String> {
        self.read_store(load_settings)
    }

    /// Remember the Telegram chat found from the owner's first message,
    /// and their first name there when no name is set yet (and it is a
    /// valid one).
    pub fn save_telegram_chat(
        &self,
        chat: &str,
        first_name: Option<&str>,
    ) -> std::result::Result<(), String> {
        let mut settings = self.brief_settings()?;
        settings.telegram_chat = chat.to_owned();
        if settings.owner_name.is_empty()
            && let Some(name) = first_name
        {
            let named = BriefSettings {
                owner_name: name.to_owned(),
                ..settings.clone()
            };
            if let Ok(named) = validate(named) {
                settings = named;
            }
        }
        let settings = validate(settings).map_err(|e| e.to_string())?;
        self.with_connection(|c| save_settings(c, &settings))
    }

    pub fn save_brief_settings(
        &self,
        settings: BriefSettings,
    ) -> std::result::Result<BriefSettings, String> {
        let mut settings = validate(settings).map_err(|e| e.to_string())?;
        // The chat is found by the server from the owner's first message;
        // a form that does not know it yet keeps it.
        // So is the name taken from Telegram.
        if settings.telegram_chat.is_empty() || settings.owner_name.is_empty() {
            let saved = self.brief_settings()?;
            if settings.telegram_chat.is_empty() {
                settings.telegram_chat = saved.telegram_chat;
            }
            if settings.owner_name.is_empty() {
                settings.owner_name = saved.owner_name;
            }
        }
        self.with_connection(|c| save_settings(c, &settings))?;
        crate::rafael::set_persona(settings.persona.name());
        Ok(settings)
    }

    pub fn latest_brief(&self) -> std::result::Result<Option<Brief>, String> {
        self.read_store(|c| {
            let has: bool = c.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'voice_briefs')",
                [],
                |r| r.get(0),
            )?;
            if has { latest(c) } else { Ok(None) }
        })
    }

    /// The spoken brief of local `date` (`YYYY-MM-DD`), if it has audio.
    pub fn brief_audio(&self, date: &str) -> std::result::Result<Option<Vec<u8>>, String> {
        if date.parse::<Date>().is_err() {
            return Err("Not a date.".into());
        }
        self.read_store(|c| audio(c, date))
    }

    pub fn brief_facts_at(&self, now: Timestamp) -> std::result::Result<BriefFacts, String> {
        self.read_forecast_at(now, |store, snap, zoned, model| {
            let report = crate::dashboard::dashboard(store, snap, zoned, model)?;
            let ticks = crate::day_summary::load_ticks(store.connection())?;
            Ok(facts(
                &crate::day_summary::limits(&report),
                &report.subscriptions.scheduled_payments,
                snap,
                zoned,
                &ticks,
            ))
        })
    }

    /// Today's brief as `settings` say, without saving or sending it:
    /// the brief and its audio. `voices` is asked only when the mood
    /// should follow the day.
    pub fn preview_brief(
        &self,
        now: Timestamp,
        settings: &BriefSettings,
        writer: &dyn BriefWriter,
        speaker: &dyn Speaker,
        voices: &dyn Fn() -> Vec<VoiceOption>,
    ) -> std::result::Result<(Brief, Option<Vec<u8>>), String> {
        let settings = validate(settings.clone()).map_err(|e| e.to_string())?;
        let facts = self.brief_facts_at(now)?;
        let (Written { title, script, gif }, source) = compose(&facts, &settings, writer);
        let mut voice = String::new();
        let audio = if settings.voice {
            let list = if settings.match_mood {
                voices()
            } else {
                Vec::new()
            };
            voice = pick_voice(&settings, &list, &facts);
            match speaker.speak(&script, &voice, settings.expressiveness()) {
                Ok(a) => Some(a),
                Err(e) => {
                    eprintln!("brief: no audio ({e})");
                    None
                }
            }
        } else {
            None
        };
        if audio.is_none() {
            voice.clear();
        }
        let brief = Brief {
            date: facts.date.clone(),
            created_at: isoformat_utc(now),
            title,
            script,
            source: source.into(),
            has_audio: audio.is_some(),
            facts,
            voice_id: voice,
            persona: settings.persona.name().into(),
            gif,
        };
        Ok((brief, audio))
    }

    /// Write, speak and save today's brief with the saved settings now;
    /// `deliver` sends it.
    pub fn make_brief(
        &self,
        now: Timestamp,
        writer: &dyn BriefWriter,
        speaker: &dyn Speaker,
        voices: &dyn Fn() -> Vec<VoiceOption>,
        deliver: &dyn Fn(&Brief) -> bool,
    ) -> std::result::Result<Brief, String> {
        let settings = self.brief_settings()?;
        let (brief, audio) = self.preview_brief(now, &settings, writer, speaker, voices)?;
        self.with_connection(|c| save(c, &brief, audio.as_deref()))?;
        deliver(&brief);
        Ok(brief)
    }

    /// The scheduler: once per daily slot at the brief's hour. `Ok(true)`
    /// when a brief was made.
    pub fn run_brief(
        &self,
        now: Timestamp,
        writer: &dyn BriefWriter,
        speaker: &dyn Speaker,
        voices: &dyn Fn() -> Vec<VoiceOption>,
        deliver: &dyn Fn(&Brief) -> bool,
    ) -> std::result::Result<bool, String> {
        let settings = self.brief_settings()?;
        if !settings.enabled {
            return Ok(false);
        }
        let zone = self.read_snapshot(|s| s.zone.clone())?;
        let due = latest_due(now, &zone, settings.hour);
        if !self.with_connection(|c| claim(c, JOB, now, due))? {
            return Ok(false);
        }
        self.make_brief(now, writer, speaker, voices, deliver)?;
        self.with_connection(|c| complete(c, JOB, now))?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "brief_tests.rs"]
mod tests;
