//! `cn()`: join class lists so a later utility replaces an earlier one of the
//! same kind, as `tailwind-merge` does on the site. Without it both classes
//! would apply and CSS order, not call order, would decide.

/// The conflict group of one utility (variants included), or `None` to keep it.
fn group(class: &str) -> Option<String> {
    let (variants, utility) = match class.rfind(':') {
        Some(i) if !class[..i].contains('[') || class[..i].contains(']') => {
            (&class[..=i], &class[i + 1..])
        }
        _ => ("", class),
    };
    let utility = utility.trim_start_matches('!').trim_start_matches('-');
    let base = utility.split('/').next().unwrap_or(utility);
    const TEXT_SIZES: [&str; 9] = [
        "eyebrow", "xs", "sm", "base", "lg", "xl", "2xl", "display", "3xl",
    ];
    const FONT_WEIGHTS: [&str; 6] = ["normal", "medium", "semibold", "bold", "light", "extrabold"];
    let kind = if let Some(rest) = base.strip_prefix("text-") {
        if TEXT_SIZES.contains(&rest) {
            "text-size".to_owned()
        } else if ["left", "right", "center", "justify"].contains(&rest) {
            "text-align".to_owned()
        } else {
            "text-color".to_owned()
        }
    } else if let Some(rest) = base.strip_prefix("font-") {
        if FONT_WEIGHTS.contains(&rest) {
            "font-weight".to_owned()
        } else {
            "font-family".to_owned()
        }
    } else if let Some(rest) = base.strip_prefix("border-") {
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit())
            || ["x", "y", "t", "b", "l", "r"].contains(&rest)
            || rest.starts_with("t-")
            || rest.starts_with("b-")
            || rest.starts_with("l-")
            || rest.starts_with("r-")
        {
            format!("border-w-{}", rest.split('-').next().unwrap_or(""))
        } else if ["dashed", "solid", "dotted", "none"].contains(&rest) {
            "border-style".to_owned()
        } else {
            "border-color".to_owned()
        }
    } else if base == "border" {
        "border-w-".to_owned()
    } else if let Some(rest) = base.strip_prefix("bg-") {
        if rest.starts_with("[length")
            || ["no-repeat", "repeat", "cover", "contain"].contains(&rest)
        {
            return None;
        }
        "bg".to_owned()
    } else if [
        "block",
        "inline-block",
        "inline",
        "flex",
        "inline-flex",
        "grid",
        "inline-grid",
        "hidden",
        "table",
        "table-cell",
        "contents",
    ]
    .contains(&base)
    {
        "display".to_owned()
    } else if ["static", "fixed", "absolute", "relative", "sticky"].contains(&base) {
        "position".to_owned()
    } else if let Some((prefix, _)) = base.split_once('-') {
        const PREFIXES: [&str; 40] = [
            "p",
            "px",
            "py",
            "pt",
            "pb",
            "pl",
            "pr",
            "m",
            "mx",
            "my",
            "mt",
            "mb",
            "ml",
            "mr",
            "w",
            "h",
            "size",
            "min-w",
            "min-h",
            "max-w",
            "max-h",
            "gap",
            "gap-x",
            "gap-y",
            "rounded",
            "shadow",
            "items",
            "justify",
            "grid-cols",
            "col-span",
            "leading",
            "tracking",
            "opacity",
            "z",
            "top",
            "left",
            "right",
            "bottom",
            "overflow",
            "self",
        ];
        let two = base.splitn(3, '-').take(2).collect::<Vec<_>>().join("-");
        if PREFIXES.contains(&two.as_str()) {
            two
        } else if PREFIXES.contains(&prefix) {
            prefix.to_owned()
        } else {
            return None;
        }
    } else if ["rounded", "shadow", "grow", "shrink", "truncate"].contains(&base) {
        base.to_owned()
    } else {
        return None;
    };
    Some(format!("{variants}{kind}"))
}

/// Groups a later `kind` also overrides (padding shorthands and friends).
fn covers(later: &str, earlier: &str) -> bool {
    if later == earlier {
        return true;
    }
    let split = |g: &str| match g.rfind(':') {
        Some(i) => (g[..=i].to_owned(), g[i + 1..].to_owned()),
        None => (String::new(), g.to_owned()),
    };
    let ((lv, lk), (ev, ek)) = (split(later), split(earlier));
    if lv != ev {
        return false;
    }
    let family = |k: &str| -> &'static [&'static str] {
        match k {
            "p" => &["px", "py", "pt", "pb", "pl", "pr"],
            "px" => &["pl", "pr"],
            "py" => &["pt", "pb"],
            "m" => &["mx", "my", "mt", "mb", "ml", "mr"],
            "mx" => &["ml", "mr"],
            "my" => &["mt", "mb"],
            "size" => &["w", "h"],
            "gap" => &["gap-x", "gap-y"],
            _ => &[],
        }
    };
    family(&lk).contains(&ek.as_str())
}

/// Join class lists, later conflicting utilities winning.
pub fn cn<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut kept: Vec<(&str, Option<String>)> = Vec::new();
    for part in parts {
        for class in part.split_whitespace() {
            let g = group(class);
            if let Some(g) = &g {
                kept.retain(|(_, other)| other.as_ref().is_none_or(|o| !covers(g, o)));
            }
            kept.push((class, g));
        }
    }
    kept.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::cn;

    #[test]
    fn later_wins() {
        assert_eq!(
            cn(["p-5 sm:p-6 text-fg", "p-4 sm:p-4"]),
            "text-fg p-4 sm:p-4"
        );
        assert_eq!(cn(["px-2 py-2.5", "px-1.5"]), "py-2.5 px-1.5");
        assert_eq!(cn(["text-sm text-fg", "text-xs"]), "text-fg text-xs");
        assert_eq!(
            cn(["text-sm text-fg", "text-fg-muted"]),
            "text-sm text-fg-muted"
        );
        assert_eq!(
            cn(["border border-border", "border-transparent"]),
            "border border-transparent"
        );
        assert_eq!(cn(["h-8 px-3", "size-9 px-0"]), "size-9 px-0");
    }
}
