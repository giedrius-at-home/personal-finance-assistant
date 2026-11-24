//! Tauri denies any app command without an ACL permission, so every command
//! the app registers must be in build.rs `COMMANDS` and allowed by the
//! default capability. Read as text: build.rs and main.rs are not importable.

const MAIN: &str = include_str!("../src/main.rs");
const BUILD: &str = include_str!("../build.rs");
const CAPABILITY: &str = include_str!("../capabilities/default.json");

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text.find(start).expect("start marker") + start.len();
    let len = text[from..].find(end).expect("end marker");
    &text[from..from + len]
}

#[test]
fn every_registered_command_is_listed_and_allowed() {
    let registered: Vec<&str> = between(MAIN, "generate_handler![", "]")
        .split(',')
        .map(|c| c.trim().rsplit("::").next().unwrap_or_default())
        .filter(|c| !c.is_empty())
        .collect();
    assert!(registered.len() > 50, "parsed {registered:?}");
    let listed: Vec<&str> = between(BUILD, "const COMMANDS: &[&str] = &[", "];")
        .split(',')
        .map(|c| c.trim().trim_matches('"'))
        .filter(|c| !c.is_empty())
        .collect();
    for command in registered {
        assert!(
            listed.contains(&command),
            "{command} missing from build.rs COMMANDS"
        );
        let allow = format!("\"allow-{}\"", command.replace('_', "-"));
        assert!(
            CAPABILITY.contains(&allow),
            "{allow} missing from capabilities/default.json"
        );
    }
}
