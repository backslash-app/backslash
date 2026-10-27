//! Markdown subset → Pango markup for detail views.
//!
//! Supports headings, paragraphs, `**bold**`, `*italic*`, `` `code` ``,
//! fenced code blocks, `[links](url)`, bullet and numbered lists, `> quotes`
//! and `---`. Anything else is shown as plain text, escaped.

use gtk::glib;

pub fn to_pango(markdown: &str) -> String {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut code = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                out.push(format!(
                    "<span font_family=\"monospace\" background=\"#808080\" bgalpha=\"18%\">{}</span>",
                    glib::markup_escape_text(&code.join("\n"))
                ));
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push(line.to_string());
            continue;
        }
        let trimmed = line.trim_start();
        let rendered = if let Some((level, text)) = heading(trimmed) {
            let size = ["xx-large", "x-large", "large"][level.min(3) - 1];
            format!("<span size=\"{size}\" weight=\"bold\">{}</span>", inline(text))
        } else if trimmed == "---" || trimmed == "***" {
            "<span alpha=\"40%\">────────────────────────</span>".to_string()
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let indent = "  ".repeat((line.len() - trimmed.len()) / 2);
            format!("{indent}  •  {}", inline(rest))
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            format!("<span alpha=\"70%\"><i>┃ {}</i></span>", inline(rest))
        } else if let Some((n, rest)) = numbered(trimmed) {
            format!("  {n}.  {}", inline(rest))
        } else {
            inline(line)
        };
        out.push(rendered);
    }
    if in_code && !code.is_empty() {
        out.push(format!("<tt>{}</tt>", glib::markup_escape_text(&code.join("\n"))));
    }
    out.join("\n")
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&level).then(|| line[level..].strip_prefix(' ')).flatten().map(|t| (level, t))
}

fn numbered(line: &str) -> Option<(&str, &str)> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    line[digits..].strip_prefix(". ").map(|rest| (&line[..digits], rest))
}

/// Inline spans. Escapes first, then converts markers, so text can never inject markup.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let (mut bold, mut italic) = (false, false);
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '`') {
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                out.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&code)));
                i += end + 2;
                continue;
            }
        }
        if c == '*' && next == Some('*') {
            out.push_str(if bold { "</b>" } else { "<b>" });
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_') && (italic || next.is_some_and(|n| !n.is_whitespace())) {
            out.push_str(if italic { "</i>" } else { "<i>" });
            italic = !italic;
            i += 1;
            continue;
        }
        if c == '[' {
            let rest: String = chars[i..].iter().collect();
            if let Some((label, url, used)) = link(&rest) {
                out.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    glib::markup_escape_text(url),
                    glib::markup_escape_text(label)
                ));
                i += used;
                continue;
            }
        }
        out.push_str(&glib::markup_escape_text(&c.to_string()));
        i += 1;
    }
    if italic {
        out.push_str("</i>");
    }
    if bold {
        out.push_str("</b>");
    }
    out
}

/// Parses `[label](url)` at the start of `s`; returns the char count consumed.
fn link(s: &str) -> Option<(&str, &str, usize)> {
    let close = s.find("](")?;
    let end = s[close + 2..].find(')')? + close + 2;
    let label = &s[1..close];
    let url = &s[close + 2..end];
    if label.contains('[') || url.contains(' ') {
        return None;
    }
    Some((label, url, s[..=end].chars().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<a>` is a GtkLabel extension; everything else must be valid Pango markup.
    fn assert_valid(markup: &str) {
        let without_links = markup.replace("</a>", "");
        let mut out = String::new();
        let mut rest = without_links.as_str();
        while let Some(start) = rest.find("<a href=") {
            out.push_str(&rest[..start]);
            rest = &rest[start + rest[start..].find('>').unwrap() + 1..];
        }
        out.push_str(rest);
        if let Err(e) = gtk::pango::parse_markup(&out, '\0') {
            panic!("{e}: {markup}");
        }
    }

    #[test]
    fn converts_common_markdown() {
        let md = "# Title\nSome **bold** and *it* with `a<b>`\n- item [site](https://x.dev)\n```\nlet x = 1 < 2;\n```";
        let p = to_pango(md);
        assert!(p.contains("<span size=\"xx-large\" weight=\"bold\">Title</span>"));
        assert!(p.contains("<b>bold</b>"));
        assert!(p.contains("<i>it</i>"));
        assert!(p.contains("<tt>a&lt;b&gt;</tt>"));
        assert!(p.contains("<a href=\"https://x.dev\">site</a>"));
        assert!(p.contains("let x = 1 &lt; 2;"));
        assert_valid(&p);
    }

    #[test]
    fn escapes_markup_injection() {
        let p = to_pango("<span foreground='red'>x</span> & 2 * 3");
        assert!(!p.contains("<span foreground"));
        assert_valid(&p);
    }
}
