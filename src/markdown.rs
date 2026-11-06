//! Tiny markdown subset for comments (std only, unit-testable):
//! `# `/`## `/`### ` headings, `- ` or `* ` bullets, `**bold**`, `*italic*`,
//! `` `code` ``, `~~strike~~`, and `\` to escape a marker.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    /// 0 = normal text, 1..=3 = heading level.
    pub heading: u8,
    pub bullet: bool,
    pub spans: Vec<Span>,
}

pub fn parse(src: &str) -> Vec<Line> {
    src.split('\n').map(parse_line).collect()
}

fn parse_line(line: &str) -> Line {
    let t = line.trim_end_matches('\r');
    let mut out = Line::default();
    let rest = if let Some(r) = t.strip_prefix("### ") {
        out.heading = 3;
        r
    } else if let Some(r) = t.strip_prefix("## ") {
        out.heading = 2;
        r
    } else if let Some(r) = t.strip_prefix("# ") {
        out.heading = 1;
        r
    } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        out.bullet = true;
        r
    } else {
        t
    };
    inline(rest, &Span::default(), &mut out.spans);
    out
}

fn push(out: &mut Vec<Span>, style: &Span, text: &mut String) {
    if !text.is_empty() {
        out.push(Span { text: std::mem::take(text), ..style.clone() });
    }
}

fn inline(s: &str, style: &Span, out: &mut Vec<Span>) {
    let mut buf = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap();
        // Escaped marker: keep the character literally.
        if c == '\\' {
            if let Some(n) = rest[1..].chars().next().filter(|n| "*`~\\#-".contains(*n)) {
                buf.push(n);
                i += 1 + n.len_utf8();
                continue;
            }
        }
        let mut matched = false;
        for marker in ["**", "~~", "`", "*"] {
            if !rest.starts_with(marker) {
                continue;
            }
            let after = &rest[marker.len()..];
            // Like real markdown: no space right after the opener or before the closer.
            if after.chars().next().is_none_or(|n| n.is_whitespace()) {
                continue;
            }
            let Some(end) = after
                .match_indices(marker)
                .map(|(e, _)| e)
                .find(|&e| e > 0 && !after[..e].ends_with(char::is_whitespace))
            else {
                continue;
            };
            push(out, style, &mut buf);
            let inner = &after[..end];
            match marker {
                "`" => out.push(Span { text: inner.to_string(), code: true, ..style.clone() }),
                "**" => inline(inner, &Span { bold: true, ..style.clone() }, out),
                "~~" => inline(inner, &Span { strike: true, ..style.clone() }, out),
                _ => inline(inner, &Span { italic: true, ..style.clone() }, out),
            }
            i += marker.len() * 2 + end;
            matched = true;
            break;
        }
        if !matched {
            buf.push(c);
            i += c.len_utf8();
        }
    }
    push(out, style, &mut buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Line) -> Vec<(&str, bool, bool, bool, bool)> {
        l.spans.iter().map(|s| (s.text.as_str(), s.bold, s.italic, s.code, s.strike)).collect()
    }

    #[test]
    fn plain_and_lines() {
        let v = parse("hello\nworld");
        assert_eq!(v.len(), 2);
        assert_eq!(texts(&v[0]), vec![("hello", false, false, false, false)]);
        assert_eq!(parse("").len(), 1);
        assert!(parse("")[0].spans.is_empty());
    }

    #[test]
    fn inline_styles() {
        let v = parse("a **b** *c* `d` ~~e~~ f");
        assert_eq!(
            texts(&v[0]),
            vec![
                ("a ", false, false, false, false),
                ("b", true, false, false, false),
                (" ", false, false, false, false),
                ("c", false, true, false, false),
                (" ", false, false, false, false),
                ("d", false, false, true, false),
                (" ", false, false, false, false),
                ("e", false, false, false, true),
                (" f", false, false, false, false),
            ]
        );
    }

    #[test]
    fn nesting_code_literal_and_unclosed() {
        let v = parse("**a *b* c**");
        assert_eq!(
            texts(&v[0]),
            vec![("a ", true, false, false, false), ("b", true, true, false, false), (" c", true, false, false, false)]
        );
        // Markers inside code stay literal.
        assert_eq!(texts(&parse("`a*b*`")[0]), vec![("a*b*", false, false, true, false)]);
        // Unclosed markers are plain text.
        assert_eq!(texts(&parse("2 * 3 and **x")[0]), vec![("2 * 3 and **x", false, false, false, false)]);
        // Escapes.
        assert_eq!(texts(&parse("\\*no\\*")[0]), vec![("*no*", false, false, false, false)]);
        // Non-ASCII is not split.
        assert_eq!(texts(&parse("**è**")[0]), vec![("è", true, false, false, false)]);
    }

    #[test]
    fn headings_and_bullets() {
        let v = parse("# Title\n## Sub\n### Small\n- item **x**\n* other\n#nospace");
        assert_eq!(v[0].heading, 1);
        assert_eq!(v[1].heading, 2);
        assert_eq!(v[2].heading, 3);
        assert!(v[3].bullet && v[3].spans.len() == 2);
        assert!(v[4].bullet);
        assert_eq!(v[5].heading, 0);
        assert_eq!(v[0].spans[0].text, "Title");
    }
}
