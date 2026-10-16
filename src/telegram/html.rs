//! Telegram's HTML: data escaped on its way into a template, and Claude's small markup turned
//! into tags here — no tag ever comes from the model, so nothing it writes can break a message.

/// Text for Telegram's HTML: `&`, `<` and `>` escaped.
pub fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// HTML back to the text it shows: tags out, entities back — for when Telegram will not parse it.
pub fn plain_of(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

/// Claude's answer as Telegram HTML. It may write `**bold**`, `` `code` ``, lines that start
/// `• `, `→ ` or `> `; a Markdown heading, a `- ` item or a code fence that slips through is
/// read for what it meant. Everything else is text, escaped.
pub fn from_markup(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out: Vec<String> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    let mut fence: Option<Vec<String>> = None;
    let mut blank = 0;
    let flush = |quote: &mut Vec<String>, out: &mut Vec<String>| {
        if quote.is_empty() {
            return;
        }
        let tag = if quote.len() > 4 { "<blockquote expandable>" } else { "<blockquote>" };
        out.push(format!("{tag}{}</blockquote>", quote.join("\n")));
        quote.clear();
    };
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(code) = fence.as_mut() {
            if line.trim_start().starts_with("```") {
                out.push(format!("<pre>{}</pre>", esc(&code.join("\n"))));
                fence = None;
            } else {
                code.push(line.to_string());
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            flush(&mut quote, &mut out);
            fence = Some(Vec::new());
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut quote, &mut out);
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push(String::new());
            }
            continue;
        }
        blank = 0;
        if let Some(rest) = line.strip_prefix("> ").or_else(|| (line == ">").then_some("")) {
            quote.push(inline(rest));
            continue;
        }
        flush(&mut quote, &mut out);
        let trimmed = line.trim_start();
        let heading = trimmed.trim_start_matches('#');
        if heading.len() < trimmed.len() && heading.starts_with(' ') {
            out.push(format!("<b>{}</b>", inline(heading.trim())));
        } else if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            out.push(format!("• {}", inline(rest)));
        } else if trimmed.starts_with("→ ") {
            out.push(format!("<i>{}</i>", inline(trimmed)));
        } else {
            out.push(inline(line));
        }
    }
    if let Some(code) = fence {
        out.push(format!("<pre>{}</pre>", esc(&code.join("\n"))));
    }
    flush(&mut quote, &mut out);
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Claude's answer laid out for a phone: its first line — the answer — in bold and apart from the
/// facts under it, the suggestion apart from them too; the rest as [`from_markup`] makes it.
pub fn answer(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    for (k, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let kind = if trimmed.starts_with('•') || trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            '•'
        } else if trimmed.starts_with('→') {
            '→'
        } else if trimmed.starts_with('>') {
            '>'
        } else {
            'p'
        };
        // A gap where the kind of line changes: the answer, the facts, the suggestion.
        if previous.is_some_and(|p| p != kind) {
            out.push(String::new());
        }
        if k == 0 && kind == 'p' {
            out.push(format!("<b>{}</b>", inline(&trimmed.replace("**", ""))));
        } else {
            out.push(from_markup(line));
        }
        previous = Some(kind);
    }
    out.join("\n")
}

/// One line: escaped, then `` `code` `` spans (nothing inside them is markup), then `**bold**`
/// in what is left. An unmatched mark stays as it was written.
fn inline(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let Some(open) = rest.find('`') else {
            out.push_str(&bold(&esc(rest)));
            break;
        };
        let Some(close) = rest[open + 1..].find('`') else {
            out.push_str(&bold(&esc(rest)));
            break;
        };
        out.push_str(&bold(&esc(&rest[..open])));
        let code = &rest[open + 1..open + 1 + close];
        if code.is_empty() {
            out.push_str("``");
        } else {
            out.push_str(&format!("<code>{}</code>", esc(code)));
        }
        rest = &rest[open + close + 2..];
    }
    out
}

fn bold(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("**") {
        let Some(close) = rest[open + 2..].find("**") else {
            break;
        };
        let inner = &rest[open + 2..open + 2 + close];
        if inner.trim().is_empty() {
            out.push_str(&rest[..open + 4 + close]);
        } else {
            out.push_str(&rest[..open]);
            out.push_str(&format!("<b>{inner}</b>"));
        }
        rest = &rest[open + 4 + close..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_becomes_tags_and_nothing_else_does() {
        assert_eq!(from_markup("**clickhouse3** is full: 58.2 of 64.0 GiB · 91%"), "<b>clickhouse3</b> is full: 58.2 of 64.0 GiB · 91%");
        assert_eq!(from_markup("• query `c3e51f3b` <script> a & b"), "• query <code>c3e51f3b</code> &lt;script&gt; a &amp; b");
        assert_eq!(from_markup("→ kill it"), "<i>→ kill it</i>");
        assert_eq!(from_markup("an `unclosed tick and **half bold"), "an `unclosed tick and **half bold");
        assert_eq!(from_markup("**bold** with_under_scores"), "<b>bold</b> with_under_scores");
        assert_eq!(from_markup("`**not bold**`"), "<code>**not bold**</code>");
        assert_eq!(from_markup("## Heading\n- item"), "<b>Heading</b>\n• item");
        assert_eq!(from_markup("```sql\nSELECT 1 < 2\n```"), "<pre>SELECT 1 &lt; 2</pre>");
        assert_eq!(from_markup("> one\n> two\nafter"), "<blockquote>one\ntwo</blockquote>\nafter");
        assert_eq!(from_markup("a\n\n\n\nb\n\n"), "a\n\nb");
    }

    #[test]
    fn an_answer_has_its_headline_its_facts_and_its_suggestion_apart() {
        let said = "**clickhouse3** is the fullest: 58.2 of 64.0 GiB · 91%\n• 🔴 memory 91%\n• 🟠 query `c3e5`\n→ kill it";
        assert_eq!(answer(said), "<b>clickhouse3 is the fullest: 58.2 of 64.0 GiB · 91%</b>\n\n• 🔴 memory 91%\n• 🟠 query <code>c3e5</code>\n\n<i>→ kill it</i>");
        assert_eq!(answer("just one line"), "<b>just one line</b>");
    }

    #[test]
    fn html_reads_back_as_its_text() {
        assert_eq!(plain_of("<b>ch-1</b> &lt;3 &amp; <code>x</code>"), "ch-1 <3 & x");
    }
}
