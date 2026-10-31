//! Small local command suggestions. They never execute commands or access history.
pub(super) const COMMANDS: &[(&str, &str)] = &[
    ("git checkout", "Switch branches or restore files"),
    ("git cherry-pick", "Apply selected commits"),
    ("git status", "Show working tree changes"),
    ("git branch", "List or manage branches"),
    ("git diff", "Show changes"),
    ("git log", "Show commit history"),
    ("cargo build", "Build the project"),
    ("cargo test", "Run project tests"),
    ("cargo check", "Check the project"),
    ("npm install", "Install dependencies"),
    ("npm run", "Run a package script"),
    ("docker ps", "List running containers"),
];

pub(super) fn suggestions(input: &str) -> Vec<(&'static str, &'static str)> {
    if input.len() < 3 || input.len() > 64 || !input.is_ascii() || input.chars().any(|c| c.is_ascii_control()) { return Vec::new(); }
    COMMANDS.iter().copied().filter(|(command, _)| command.starts_with(input) && *command != input).take(5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suggestions_are_local_prefixes_and_never_control_sequences() {
        assert_eq!(suggestions("git ch").iter().map(|x| x.0).collect::<Vec<_>>(), vec!["git checkout", "git cherry-pick"]);
        for input in ["", "gi", "git\n", "git \x1b", "git checkout", "ümlaut"] { assert!(suggestions(input).is_empty()); }
    }
}
