//! Window rules: where an application opens, by its identity.
//!
//! `~/.config/des-ui/rules` has one rule per line: a pattern for the Wayland app ID or
//! X11 class, with `*` and `?` as wildcards, then any of `workspace=<1-4>`,
//! `display=<1-n>` and `side`. Lines starting with `#` are comments. The first rule
//! that matches a new window wins, and it is applied once, when the window opens;
//! nothing is moved afterwards.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rule {
    pub pattern: String,
    /// The workspace to open on, counted from 0.
    pub workspace: Option<usize>,
    /// The display to open on, counted from 0.
    pub display: Option<usize>,
    /// In the right half of the workspace, beside what is there.
    pub side: bool,
}

/// Where the rules are: beside the configuration file.
pub fn path() -> Option<PathBuf> {
    Some(des_common::input::config_path()?.with_file_name("rules"))
}

/// The rules in force, read afresh so that an edit counts for the next window.
pub fn load() -> Vec<Rule> {
    path().and_then(|path| std::fs::read_to_string(path).ok()).map(|text| parse(&text)).unwrap_or_default()
}

/// Reads rules, one per line. A line that is not understood is left out, so one
/// mistake does not disable the others.
pub fn parse(text: &str) -> Vec<Rule> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Rule> {
    let line = line.split('#').next().unwrap_or("").trim();
    let mut words = line.split_whitespace();
    let pattern = words.next()?;
    let mut rule = Rule { pattern: pattern.to_string(), ..Rule::default() };
    for word in words {
        match word.split_once('=') {
            Some(("workspace", n)) => {
                let n: usize = n.parse().ok()?;
                rule.workspace = Some(n.checked_sub(1).filter(|n| *n < des_common::ipc::WORKSPACES)?);
            }
            Some(("display", n)) => {
                let n: usize = n.parse().ok()?;
                rule.display = Some(n.checked_sub(1).filter(|n| *n < 32)?);
            }
            None if word == "side" => rule.side = true,
            _ => return None,
        }
    }
    let says_something = rule.workspace.is_some() || rule.display.is_some() || rule.side;
    says_something.then_some(rule)
}

/// Whether `pattern` matches `text`: `*` stands for anything, `?` for one character,
/// and letter case does not count, since X11 classes vary in it.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut p, mut t) = (0, 0);
    // Where the last `*` was, and how much of the text it had taken, to try more.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((at, taken)) = star {
            p = at + 1;
            t = taken + 1;
            star = Some((at, t));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// The first rule that applies to a window of this identity.
pub fn find<'a>(rules: &'a [Rule], identity: &str) -> Option<&'a Rule> {
    rules.iter().find(|rule| glob_matches(&rule.pattern, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rules_and_leaves_out_what_it_does_not_understand() {
        let rules = parse(
            "# comment\n\
             firefox workspace=2\n\
             org.gnome.* display=2 side # the second monitor\n\
             mpv side\n\
             \n\
             bad workspace=9\n\
             worse display=0\n\
             nothing\n\
             odd colour=red\n\
             *.Slack workspace=4 display=1\n",
        );
        assert_eq!(
            rules,
            vec![
                Rule { pattern: "firefox".into(), workspace: Some(1), display: None, side: false },
                Rule { pattern: "org.gnome.*".into(), workspace: None, display: Some(1), side: true },
                Rule { pattern: "mpv".into(), workspace: None, display: None, side: true },
                Rule { pattern: "*.Slack".into(), workspace: Some(3), display: Some(0), side: false },
            ]
        );
    }

    #[test]
    fn wildcards_match_and_case_does_not_count() {
        assert!(glob_matches("firefox", "Firefox"));
        assert!(glob_matches("org.gnome.*", "org.gnome.Nautilus"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*.Slack", "com.slack.Slack"));
        assert!(glob_matches("a*b*c", "axxbyyc"));
        assert!(glob_matches("a*b*c", "abc"));
        assert!(glob_matches("mp?", "mpv"));
        assert!(!glob_matches("mp?", "mp"));
        assert!(!glob_matches("firefox", "firefox-esr"));
        assert!(!glob_matches("org.gnome.*", "org.kde.dolphin"));
        assert!(!glob_matches("a*b", "acb-"));
        assert!(!glob_matches("", "x"));
        assert!(glob_matches("", ""));
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let rules = parse("org.gnome.* workspace=2\norg.gnome.Nautilus workspace=3\n* side\n");
        assert_eq!(find(&rules, "org.gnome.Nautilus").map(|r| r.workspace), Some(Some(1)));
        assert_eq!(find(&rules, "mpv").map(|r| r.side), Some(true));
        assert!(find(&parse("firefox side\n"), "chromium").is_none());
    }
}
