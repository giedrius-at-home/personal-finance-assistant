use std::collections::HashMap;

use finance_types::notify::Persona;
use finance_types::report::{Currency, UnitMoney};
use jiff::Zoned;
use rust_decimal::Decimal;

use super::*;
use crate::closure::tests::{account, snapshot};
use crate::model::Tx;
use crate::trends::tests::{mapping, out};

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn weeks(values: &[&str]) -> Vec<Decimal> {
    values.iter().map(|v| dec(v)).collect()
}

/// Wednesday 25 November, 08:00: six days left in the month.
fn now() -> Zoned {
    "2026-11-25T08:00:00+01:00[Europe/Amsterdam]"
        .parse()
        .unwrap()
}

fn at(day: &str) -> String {
    format!("{day}T12:00:00+01:00[Europe/Amsterdam]")
}

/// Coffee every week: 40 a week for four weeks, then 20 in the last seven
/// days (and 20 the week before: two weeks lower in a row). Groceries
/// steady at 60.
fn snap() -> Snapshot {
    let mut txs: Vec<Tx> = Vec::new();
    // The last seven days are 18–24 November; the weeks before step back by 7.
    let coffee = [
        ("2026-11-24", "20"),
        ("2026-11-17", "25"),
        ("2026-11-10", "40"),
        ("2026-11-03", "40"),
        ("2026-10-27", "40"),
    ];
    for (i, (day, amount)) in coffee.iter().enumerate() {
        txs.push(out(&format!("c{i}"), amount, Some("Coffee"), &at(day)));
        txs.push(out(&format!("g{i}"), "60", Some("Jumbo"), &at(day)));
    }
    // Today's purchase is not part of any week yet.
    txs.push(out("today", "9", Some("Coffee"), &at("2026-11-25")));
    let mut snap = snapshot(vec![account("firefly:1")], txs, &[]);
    snap.group_mappings = vec![mapping("Coffee", "Coffee"), mapping("Jumbo", "Groceries")];
    snap
}

fn group(name: &str, limit: &str, spent: &str) -> GroupAllocation {
    GroupAllocation {
        group: name.into(),
        limit: dec(limit),
        spent: dec(spent),
        remaining: dec(limit) - dec(spent),
        projected_month_end: Decimal::ZERO,
    }
}

fn limits() -> Vec<(Currency, Vec<GroupAllocation>)> {
    vec![(
        Currency::eur(),
        vec![
            group("Coffee", "100", "90"),
            group("Groceries", "300", "150"),
            group("Going out", "50", "60"),
        ],
    )]
}

fn bill(name: &str, due: &str, paid: Option<&str>) -> ScheduledBillPayment {
    ScheduledBillPayment {
        bill_id: name.into(),
        name: name.into(),
        due_on: due.into(),
        paid_on: paid.map(str::to_owned),
        amount: UnitMoney {
            amount: dec("950"),
            currency: Currency::eur(),
        },
        candidate_ids: Vec::new(),
    }
}

fn today_facts() -> BriefFacts {
    facts(
        &limits(),
        &[
            bill("Rent", "2026-11-27", None),
            bill("Gym", "2026-11-24", Some("2026-11-24")),
            bill("Insurance", "2026-12-10", None),
        ],
        &snap(),
        &now(),
        &HashMap::new(),
    )
}

#[test]
fn money_is_said_in_whole_units() {
    assert_eq!(say_money(dec("412.49"), &Currency::eur()), "€412");
    assert_eq!(say_money(dec("1250"), &Currency::eur()), "€1,250");
    let sek = Currency {
        code: "SEK".into(),
        minor_unit: 2,
    };
    assert_eq!(say_money(dec("12500.6"), &sek), "12,501 SEK");
}

#[test]
fn a_habit_going_down_two_weeks_in_a_row() {
    let h = habit(
        "Coffee",
        &weeks(&["20", "25", "40", "40", "40"]),
        &Currency::eur(),
    )
    .unwrap();
    assert_eq!(h.direction, "down");
    assert_eq!(h.last_week, "€20");
    assert_eq!(h.usual_week, "€36");
    assert_eq!(h.change_percent, -45);
    assert_eq!(h.weeks_in_a_row, 2);
}

#[test]
fn a_habit_that_is_not_changing() {
    let h = habit(
        "Eating out",
        &weeks(&["62", "58", "60", "61", "59"]),
        &Currency::eur(),
    )
    .unwrap();
    assert_eq!(h.direction, "steady");
    assert_eq!(h.weeks_in_a_row, 5);
    let up = habit(
        "Shopping",
        &weeks(&["120", "90", "60", "60", "60"]),
        &Currency::eur(),
    )
    .unwrap();
    assert_eq!(up.direction, "up");
    assert_eq!(up.weeks_in_a_row, 2);
    assert!(habit("Tiny", &weeks(&["2", "3", "1", "0", "2"]), &Currency::eur()).is_none());
}

#[test]
fn facts_name_what_is_left_the_limits_bills_and_habits() {
    let f = today_facts();
    assert_eq!(f.date, "2026-11-25");
    assert_eq!(f.weekday, "Wednesday");
    // 10 + 150 + 0 left over six days.
    assert_eq!(f.left[0].left, "€160");
    assert_eq!(f.left[0].per_day, "€27");
    assert_eq!(f.left[0].days_left, 6);
    assert_eq!(f.over_limit[0].group, "Going out");
    assert_eq!(f.near_limit[0].group, "Coffee");
    assert_eq!(f.near_limit[0].used_percent, 90);
    // Only the unpaid bill within a week.
    assert_eq!(f.bills_soon.len(), 1);
    assert_eq!(f.bills_soon[0].name, "Rent");
    assert_eq!(f.bills_soon[0].due, "Friday");
    assert_eq!(f.spent_yesterday, ["€80"]);
    // Coffee fell; groceries are steady and not named.
    assert_eq!(f.habits.len(), 1);
    assert_eq!(f.habits[0].group, "Coffee");
    assert_eq!(f.habits[0].direction, "down");
    // Coffee is avoidable by default: the whole picture follows it.
    let avoidable = f.avoidable.as_ref().unwrap();
    assert_eq!(avoidable.direction, "down");
    assert_eq!(avoidable.weeks_in_a_row, 2);
}

#[test]
fn the_model_may_only_use_numbers_from_the_facts() {
    let f = today_facts();
    let ok = r#"{"title":"€160 left, €27 a day","script":"Hola, Gisel here. Vale, you have €160 left for the next 6 days, about €27 a day. Ojo, coffee is at 90 percent, and rent of €950 goes out Friday. But hombre, your coffee habit is actually changing: €20 this week against €36 usually, two weeks in a row. Make it at home once more today; nobody needs a new life, just a slightly better Wednesday."}"#;
    let Written { title, script, gif } = parse_brief(ok, &f, BriefLength::Normal).unwrap();
    assert_eq!(title, "€160 left, €27 a day");
    assert!(script.starts_with("Hola, Gisel here."));
    assert_eq!(gif, "", "no gif asked for, none given");
    let with_gif = ok.replacen("{", r#"{"gif":"Money On  Fire","#, 1);
    assert_eq!(
        parse_brief(&with_gif, &f, BriefLength::Normal).unwrap().gif,
        "money on fire"
    );
    // A bad GIF search is dropped, the brief stays.
    let odd_gif = ok.replacen("{", r#"{"gif":"https://x.y/€160","#, 1);
    assert_eq!(
        parse_brief(&odd_gif, &f, BriefLength::Normal).unwrap().gif,
        ""
    );
    let labelled = ok.replace("nobody needs", "life lesson: nobody needs");
    assert!(parse_brief(&labelled, &f, BriefLength::Normal).is_err());
    let invented = ok.replace("€160 left for", "€175 left for");
    assert!(parse_brief(&invented, &f, BriefLength::Normal).is_err());
    assert!(
        parse_brief(
            r#"{"title":"x","script":"too short"}"#,
            &f,
            BriefLength::Normal
        )
        .is_err()
    );
    assert!(parse_brief("not json", &f, BriefLength::Normal).is_err());
}

#[test]
fn the_fixed_script_is_honest_about_habits() {
    let Written {
        title,
        script: tagged,
        gif,
    } = fixed_brief(&today_facts(), &BriefSettings::default());
    assert!(tagged.starts_with("[emotion=warm]Hola, Gisel here."));
    assert_eq!(gif, "disappointed face palm");
    let named = BriefSettings {
        owner_name: "Giedrius".into(),
        ..BriefSettings::default()
    };
    assert!(
        fixed_brief(&today_facts(), &named)
            .script
            .starts_with("[emotion=warm]Hola Giedrius, Gisel here.")
    );
    let script = plain_script(&tagged);
    assert_eq!(title, "€160 left, €27 a day");
    assert!(script.starts_with("Hola, Gisel here. Vale, you have €160 left"));
    assert!(script.contains("Ojo, Going out is over its limit"));
    assert!(script.contains("Rent of €950 goes out Friday."));
    assert!(script.contains("It is changing, bueno."));
    assert!(!script.to_lowercase().contains("lesson"));
    // Every number in the fixed script passes the model's check too.
    let reply = serde_json::json!({ "title": title, "script": tagged }).to_string();
    assert!(parse_brief(&reply, &today_facts(), BriefLength::Normal).is_ok());
}

#[test]
fn delivery_tags_reach_the_voice_and_unknown_ones_are_refused() {
    let f = today_facts();
    let script = "[emotion=warm]Hola, Gisel here. Vale, you have €160 left for the next 6 days, about €27 a day. [sigh] Ojo, coffee is at 90 percent, and rent of €950 goes out Friday. [emotion=playful]But hombre, your coffee habit is actually changing: €20 this week against €36 usually, two weeks in a row. [chuckle] Make it at home once more today; [pace=slow]nobody needs a new life, just a slightly better Wednesday.";
    let reply = |s: &str| serde_json::json!({ "title": "€160 left", "script": s }).to_string();
    let kept = parse_brief(&reply(script), &f, BriefLength::Normal)
        .unwrap()
        .script;
    assert_eq!(kept, script);
    assert_eq!(
        plain_script("[emotion=warm]Hola. [sigh] Ojo , rent [pace=slow]goes out."),
        "Hola. Ojo, rent goes out."
    );
    for bad in [
        "[emotion=furious]",
        "[whispers]",
        "[pace=warp]",
        "[note: 12]",
    ] {
        let s = script.replacen("[sigh]", bad, 1);
        assert!(
            parse_brief(&reply(&s), &f, BriefLength::Normal).is_err(),
            "{bad}"
        );
    }
    let loose = script.replacen("[sigh]", "sigh]", 1);
    assert!(parse_brief(&reply(&loose), &f, BriefLength::Normal).is_err());
    let many = format!("{}{script}", "[breath] ".repeat(6));
    assert!(parse_brief(&reply(&many), &f, BriefLength::Normal).is_err());
    let prompt = system_prompt(&BriefSettings::default(), &f);
    assert!(prompt.contains("wistful") && prompt.contains("[chuckle]") && !prompt.contains('{'));
}

struct Failing;
impl BriefWriter for Failing {
    fn write(&self, _: &BriefFacts, _: &BriefSettings) -> std::result::Result<Written, String> {
        Err("down".into())
    }
}

#[test]
fn a_failing_writer_falls_back_to_the_fixed_script() {
    let rafael = BriefSettings {
        persona: Persona::Rafael,
        ..BriefSettings::default()
    };
    let (written, source) = compose(&today_facts(), &rafael, &Failing);
    assert_eq!(source, "fixed");
    assert!(plain_script(&written.script).starts_with("Hola, Rafael here."));
}

#[test]
fn briefs_are_saved_with_audio_and_pruned() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../storage/src/brief.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../storage/src/brief_gif.sql"))
        .unwrap();
    assert_eq!(load_settings(&conn).unwrap(), BriefSettings::default());
    let brief = |date: &str| Brief {
        date: date.into(),
        created_at: "2026-11-25T07:00:00Z".into(),
        title: "t".into(),
        script: "s".into(),
        source: "fixed".into(),
        has_audio: true,
        facts: BriefFacts::default(),
        voice_id: "voice_483".into(),
        persona: "Rafael".into(),
        gif: "slow clap".into(),
    };
    save(&conn, &brief("2026-11-01"), None).unwrap();
    save(&conn, &brief("2026-11-25"), Some(b"ID3")).unwrap();
    let latest = latest(&conn).unwrap().unwrap();
    assert_eq!(latest.date, "2026-11-25");
    assert!(latest.has_audio);
    assert_eq!(latest.voice_id, "voice_483");
    assert_eq!(latest.persona, "Rafael");
    assert_eq!(latest.gif, "slow clap");
    assert_eq!(audio(&conn, "2026-11-25").unwrap().unwrap(), b"ID3");
    // More than two weeks old: gone.
    assert!(audio(&conn, "2026-11-01").unwrap().is_none());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM voice_briefs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn the_prompt_follows_persona_length_and_style() {
    let facts = today_facts();
    let gisel = system_prompt(&BriefSettings::default(), &facts);
    assert!(gisel.starts_with("You are Gisel, from Spain"));
    assert!(gisel.contains("\"Hola, Gisel here.\""));
    assert!(gisel.contains("70 to 140 words"));
    assert!(!gisel.contains('{'));
    assert!(gisel.contains("talk to the owner every morning"));
    let rafael = system_prompt(
        &BriefSettings {
            persona: Persona::Rafael,
            length: BriefLength::Short,
            style: "Be drier.".into(),
            owner_name: "Giedrius".into(),
            ..BriefSettings::default()
        },
        &facts,
    );
    assert!(rafael.starts_with("You are Rafael"));
    assert!(rafael.contains("45 to 85 words"));
    assert!(rafael.ends_with("Be drier."));
    assert!(rafael.contains("talk to Giedrius every morning"));
    assert!(rafael.contains("start with \"Hola Giedrius, Rafael here.\""));
    assert!(rafael.contains("\"gif\"") && !rafael.contains('{'));
    assert!(
        validate(BriefSettings {
            owner_name: "  Giedrius   M. ".into(),
            ..BriefSettings::default()
        })
        .unwrap()
        .owner_name
            == "Giedrius M."
    );
    for bad in ["{name}", "R2D2", &"a".repeat(MAX_OWNER_NAME + 1)] {
        assert!(
            validate(BriefSettings {
                owner_name: bad.into(),
                ..BriefSettings::default()
            })
            .is_err(),
            "{bad}"
        );
    }
    assert!(
        validate(BriefSettings {
            style: "Say 3 jokes".into(),
            ..BriefSettings::default()
        })
        .is_err()
    );
    assert!(
        validate(BriefSettings {
            style: "x".repeat(MAX_STYLE + 1),
            ..BriefSettings::default()
        })
        .is_err()
    );
}

fn voice(id: &str, slug: &str, name: &str) -> VoiceOption {
    VoiceOption {
        id: id.into(),
        slug: slug.into(),
        name: name.into(),
        ..VoiceOption::default()
    }
}

fn paul_and_elena() -> Vec<VoiceOption> {
    vec![
        voice("p-n", "paul_neutral", "Paul - Neutral"),
        voice("p-h", "paul_happy", "Paul - Happy"),
        voice("p-x", "paul_excited", "Paul - Excited"),
        voice("p-f", "paul_frustrated", "Paul - Frustrated"),
        voice("p-c", "paul_confident", "Paul - Confident"),
        voice("o-s", "gb_oliver_sad", "Oliver"),
        voice("o-c", "gb_oliver_cheerful", "Oliver"),
        voice("e", "voice_483", "Elena"),
    ]
}

#[test]
fn the_mood_follows_the_day() {
    let mut f = today_facts();
    // Going out is over its limit: low.
    assert_eq!(moods(&f)[0], "frustrated");
    f.over_limit.clear();
    // Avoidable spending down two weeks: happy.
    assert_eq!(moods(&f)[0], "happy");
    f.avoidable.as_mut().unwrap().weeks_in_a_row = 3;
    assert_eq!(moods(&f)[0], "excited");
    f.avoidable.as_mut().unwrap().direction = "steady".into();
    assert_eq!(moods(&f)[0], "confident");
    assert_eq!(moods(&f).last(), Some(&"neutral"));
}

#[test]
fn a_mood_version_of_the_chosen_voice_speaks_when_there_is_one() {
    let voices = paul_and_elena();
    let mut f = today_facts();
    f.over_limit.clear();
    let paul = BriefSettings {
        voice_id: "p-n".into(),
        ..BriefSettings::default()
    };
    assert_eq!(pick_voice(&paul, &voices, &f), "p-h");
    // Chosen by slug works too.
    let by_slug = BriefSettings {
        voice_id: "paul_neutral".into(),
        ..BriefSettings::default()
    };
    assert_eq!(pick_voice(&by_slug, &voices, &f), "p-h");
    // Over the limit: Paul is frustrated; Oliver (by slug) has no
    // frustrated version, so sad.
    let over = today_facts();
    assert_eq!(pick_voice(&paul, &voices, &over), "p-f");
    let oliver = BriefSettings {
        voice_id: "o-c".into(),
        ..BriefSettings::default()
    };
    assert_eq!(pick_voice(&oliver, &voices, &over), "o-s");
    // Elena has no versions; mood off keeps the pick.
    assert_eq!(
        pick_voice(&BriefSettings::default(), &voices, &f),
        "voice_483"
    );
    assert!(mood_versions("voice_483", &voices).is_empty());
    let off = BriefSettings {
        match_mood: false,
        ..paul
    };
    assert_eq!(pick_voice(&off, &voices, &f), "p-n");
}

#[test]
fn personas_bring_their_own_voice() {
    assert_eq!(BriefSettings::default().chosen_voice(), "voice_483");
    let rafael = BriefSettings {
        persona: Persona::Rafael,
        ..BriefSettings::default()
    };
    assert_eq!(rafael.chosen_voice(), "voice_3143");
    assert_eq!(
        BriefSettings {
            voice_id: "mine".into(),
            ..rafael
        }
        .chosen_voice(),
        "mine"
    );
    // Settings saved before personas still load, as Gisel.
    let old: BriefSettings = serde_json::from_str(r#"{"enabled":true,"hour":7}"#).unwrap();
    assert_eq!(old.persona, Persona::Gisel);
    assert_eq!(old.expressiveness, 20);
    assert!(old.match_mood);
    assert!(
        validate(BriefSettings {
            expressiveness: 45,
            ..BriefSettings::default()
        })
        .is_ok()
    );
    assert!(
        validate(BriefSettings {
            expressiveness: 51,
            ..BriefSettings::default()
        })
        .is_err()
    );
}

#[test]
fn settings_are_checked() {
    assert!(
        validate(BriefSettings {
            hour: 24,
            ..BriefSettings::default()
        })
        .is_err()
    );
    assert!(
        validate(BriefSettings {
            voice_id: "has space".into(),
            ..BriefSettings::default()
        })
        .is_err()
    );
    assert_eq!(
        validate(BriefSettings {
            voice_id: " v1 ".into(),
            ..BriefSettings::default()
        })
        .unwrap()
        .voice_id,
        "v1"
    );
    for (chat, ok) in [
        ("", true),
        (" 123456789 ", true),
        ("-1001234567890", true),
        ("@me", false),
        ("-", false),
    ] {
        let checked = validate(BriefSettings {
            telegram_chat: chat.into(),
            ..BriefSettings::default()
        });
        assert_eq!(checked.is_ok(), ok, "{chat:?}");
    }
}

#[test]
fn the_days_take_turns_between_loving_and_savage() {
    assert_eq!(Tone::of_day("2026-11-26"), Tone::Savage);
    assert_eq!(Tone::of_day("2026-11-27"), Tone::MeanButLoving);
    let mut facts = today_facts();
    let settings = BriefSettings {
        owner_name: "Giedrius".into(),
        ..BriefSettings::default()
    };
    facts.date = "2026-11-27".into();
    let loving = system_prompt(&settings, &facts);
    facts.date = "2026-11-26".into();
    let savage = system_prompt(&settings, &facts);
    assert!(loving.contains("mean but loving") && !loving.contains("full savage"));
    assert!(savage.contains("full savage") && savage.contains("roast Giedrius hard"));
}
