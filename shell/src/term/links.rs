//! HTTP links in the visible terminal grid. Never interpret text as a command.
pub(super) fn at(screen: &vt100::Screen, row: u16, col: u16) -> Option<String> {
    let (rows, cols) = screen.size();
    if row >= rows || col >= cols { return None; }
    let mut first = row;
    while first > 0 && screen.row_wrapped(first - 1) { first -= 1; }
    let mut last = row;
    while last + 1 < rows && screen.row_wrapped(last) { last += 1; }
    let mut text = String::new();
    let mut hit = 0;
    for r in first..=last {
        for c in 0..cols {
            if (r, c) == (row, col) { hit = text.len(); }
            let Some(cell) = screen.cell(r, c) else { continue; };
            if cell.is_wide_continuation() { continue; }
            let contents = cell.contents();
            if contents.is_empty() { text.push(' '); } else { text.push_str(&contents); }
        }
    }
    let start = text[..hit].rfind(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '\"' | '\''))
        .map_or(0, |i| i + text[..hit][i..].chars().next().unwrap().len_utf8());
    let end = text[hit..].find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '\"' | '\''))
        .map_or(text.len(), |i| hit + i);
    let raw = &text[start..end];
    let offset = raw.find("https://").or_else(|| raw.find("http://"))?;
    // Only strip common prose delimiters before the scheme.
    if !raw[..offset].chars().all(|c| matches!(c, '(' | '[' | '{')) { return None; }
    let mut url = raw[offset..].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        while url.ends_with(close) && url.matches(close).count() > url.matches(open).count() {
            url = &url[..url.len() - 1];
        }
    }
    if hit < start + offset || hit >= start + offset + url.len() { return None; }
    let host = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    if host.is_empty() || url.chars().any(char::is_control) { return None; }
    Some(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_urls_by_cell_and_trims_prose() {
        let mut parser = vt100::Parser::new(4, 80, 0);
        parser.process("中文 (https://example.org/a?q=1). other".as_bytes());
        assert_eq!(at(parser.screen(), 0, 10).as_deref(), Some("https://example.org/a?q=1"));
        assert_eq!(at(parser.screen(), 0, 1), None);
        assert_eq!(at(parser.screen(), 0, 35), None);
    }
    #[test]
    fn joins_soft_wrapped_urls_but_not_separate_lines() {
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(b"https://example.org/long/path\r\nhttps://next.org");
        assert_eq!(at(parser.screen(), 1, 3).as_deref(), Some("https://example.org/long/path"));
        assert_eq!(at(parser.screen(), 2, 4).as_deref(), Some("https://next.org"));
    }
    #[test]
    fn preserves_balanced_parentheses_and_rejects_other_schemes() {
        let mut parser = vt100::Parser::new(4, 80, 0);
        parser.process(b"https://example.org/a_(b) file:///tmp/a javascript:alert(1)");
        assert_eq!(at(parser.screen(), 0, 10).as_deref(), Some("https://example.org/a_(b)"));
        assert_eq!(at(parser.screen(), 0, 30), None);
        assert_eq!(at(parser.screen(), 0, 45), None);
    }
}
