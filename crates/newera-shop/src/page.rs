//! A cleaned page as lines of text, the way a person reads it.
//!
//! `AnySite`'s `webparser` hands back the page's HTML cleaned: every
//! `<script>` and `<meta>` is gone (measured on Praxis with `include_tags`
//! asking for them, and on a Hornbach render), so a product's JSON-LD and
//! `OpenGraph` never arrive. What arrives is the page as shown: a heading, a
//! price, a table of specifications. Each block element (`div`, `dt`, `dd`,
//! `li`, `td`…) starts a line and inline ones (`span`, `b`, `sup`) join
//! theirs, so `<dt>Inhoud per pak voor</dt><dd>1,62 m²</dd>` reads as two
//! lines, a label and its value, whichever markup a shop uses for its table.
//! Buttons, icons and forms are not content and are left out.
//!
//! Besides the text, two things are kept: an accessibility label that is
//! an amount (`aria-label="EUR 17.59"`), and every image with its
//! description (`alt`). Microdata is not read: the only `itemprop` the
//! recorded pages carry is `image`, and on Praxis it marks the menu's
//! pictures of other products.

/// Tags that end a line.
const BLOCK: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "section",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];
/// Tags whose content is not read.
const SKIP: &[&str] = &[
    "button", "noscript", "script", "select", "style", "svg", "template", "textarea",
];
/// Tags that strike a price out: the old price.
const STRUCK: &[&str] = &["del", "s", "strike"];
/// Tags that never close.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// An amount a page labels for screen readers: `aria-label="EUR 17.59"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labelled {
    /// The line the labelled element starts on.
    pub line: usize,
    /// The label as written.
    pub label: String,
}

/// A page as lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub lines: Vec<String>,
    /// The lines that are a first-level heading, by index.
    pub headings: Vec<usize>,
    /// Labelled amounts outside a struck-out price, in page order.
    pub labelled: Vec<Labelled>,
    /// `(alt, src)` of every image that has both, in page order; the
    /// description's spaces collapsed.
    pub images: Vec<(String, String)>,
}

impl Page {
    /// The first first-level heading.
    pub fn heading(&self) -> Option<(usize, &str)> {
        self.headings.first().map(|&i| (i, self.lines[i].as_str()))
    }

    /// The value of a label: the line after a line that is the label — a
    /// specification table — or else the rest of a line that starts with
    /// `label:`. Case is ignored, and so is a colon after the label.
    pub fn value(&self, label: &str) -> Option<(&str, String)> {
        let wanted = label.to_lowercase();
        let own_line = self.lines.iter().enumerate().find_map(|(i, line)| {
            let next = self.lines.get(i + 1)?;
            (line.to_lowercase().trim_end_matches(':').trim() == wanted)
                .then(|| (next.as_str(), format!("{line} {next}")))
        });
        own_line.or_else(|| {
            let prefix = format!("{wanted}:");
            self.lines.iter().find_map(|line| {
                let rest = strip_prefix_lowercase(line, &prefix)?;
                Some((rest.trim(), line.clone()))
            })
        })
    }

    /// The source of the first image whose description is `alt`.
    pub fn image(&self, alt: &str) -> Option<&str> {
        self.images
            .iter()
            .find(|(a, _)| a == alt)
            .map(|(_, src)| src.as_str())
    }

    pub fn read(html: &str) -> Self {
        let mut page = Self::default();
        let mut current = String::new();
        let mut skipping: Vec<String> = Vec::new();
        let mut struck = 0usize;
        let mut in_heading = false;
        let mut rest = html;
        while !rest.is_empty() {
            let Some(open) = rest.find('<') else {
                if skipping.is_empty() {
                    current.push_str(rest);
                }
                break;
            };
            if skipping.is_empty() {
                current.push_str(&rest[..open]);
            }
            rest = &rest[open..];
            if let Some(after) = rest.strip_prefix("<!--") {
                rest = after.find("-->").map_or("", |end| &after[end + 3..]);
                continue;
            }
            let end = tag_end(rest);
            // A tag the text ends inside has no `>` to cut off.
            let inner = if rest[..end].ends_with('>') {
                end - 1
            } else {
                end
            };
            let tag = &rest[1..inner.max(1)];
            rest = &rest[end..];
            let closing = tag.starts_with('/');
            let body = tag.trim_start_matches('/');
            let name: String = body
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_lowercase();
            if name.is_empty() {
                continue;
            }
            let self_closing = body.trim_end().ends_with('/') || VOID.contains(&name.as_str());
            if SKIP.contains(&name.as_str()) {
                if !self_closing {
                    if closing {
                        if skipping.last() == Some(&name) {
                            skipping.pop();
                        }
                    } else {
                        skipping.push(name);
                    }
                }
                continue;
            }
            // A gallery's pictures sit inside buttons: an image counts
            // wherever it is.
            if name == "img"
                && let Some(alt) = attribute(body, "alt")
                && let Some(src) = attribute(body, "src")
            {
                let alt = alt.split_whitespace().collect::<Vec<_>>().join(" ");
                if !alt.is_empty() {
                    page.images.push((alt, src));
                }
            }
            if !skipping.is_empty() {
                continue;
            }
            if BLOCK.contains(&name.as_str()) {
                let had = page.lines.len();
                flush(&mut current, &mut page.lines);
                if name == "h1" {
                    if closing && in_heading && page.lines.len() > had {
                        page.headings.push(page.lines.len() - 1);
                    }
                    in_heading = !closing;
                }
            }
            if STRUCK.contains(&name.as_str()) && !self_closing {
                if closing {
                    struck = struck.saturating_sub(1);
                } else {
                    struck += 1;
                }
            }
            if closing {
                continue;
            }
            let in_struck = struck > 0;
            if let Some(label) = attribute(body, "aria-label")
                && !in_struck
                && is_amount_label(&label)
            {
                let line = page.lines.len();
                page.labelled.push(Labelled { line, label });
            }
        }
        flush(&mut current, &mut page.lines);
        page
    }
}

/// What follows `prefix` (lower case) at the start of `line`, compared in
/// lower case, cut from `line` itself: a character whose lower case is
/// longer than it is (`İ`) does not move the cut.
fn strip_prefix_lowercase<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let mut matched = 0;
    for (at, c) in line.char_indices() {
        if matched >= prefix.len() {
            return Some(&line[at..]);
        }
        let lower: String = c.to_lowercase().collect();
        if !prefix[matched..].starts_with(&lower) {
            return None;
        }
        matched += lower.len();
    }
    (matched >= prefix.len()).then_some("")
}

/// Where a tag that starts `text` ends, past its `>`; a `>` inside a quoted
/// attribute does not end it.
fn tag_end(text: &str) -> usize {
    let mut quote = None;
    for (i, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '>') => return i + 1,
            _ => {}
        }
    }
    text.len()
}

/// An attribute's value, entities decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        rest = after;
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let Some(after) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let (quote, value) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => (q, &after[1..]),
            _ => (' ', after),
        };
        let end = value.find(quote).unwrap_or(value.len());
        return Some(decode(&value[..end]));
    }
    None
}

fn is_amount_label(label: &str) -> bool {
    let label = label.trim();
    let amount = label
        .strip_prefix("EUR")
        .or_else(|| label.strip_prefix('€'))
        .map(str::trim);
    amount.is_some_and(|a| crate::decimal::Decimal::parse(a).is_some())
}

fn flush(current: &mut String, lines: &mut Vec<String>) {
    let text = decode(current);
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !line.is_empty() {
        lines.push(line);
    }
    current.clear();
}

/// The entities a cleaned page uses.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let window = rest.floor_char_boundary(rest.len().min(12));
        let Some(end) = rest[..window].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "euro" => Some('€'),
            "sup2" => Some('²'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        if let Some(c) = decoded {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_make_lines_and_inline_elements_join_them() {
        let page = Page::read(
            r#"<div><h1>Kast <b>Alaska</b></h1><dl><dt>Inhoud per pak voor<button>i</button></dt><dd>1,62&nbsp;m&sup2;</dd></dl>
            <span class="p">135,</span><span>20</span><svg><path d="x"/><text>no</text></svg><!-- note --><p>Tegel: Mat</p></div>"#,
        );
        assert_eq!(
            page,
            Page {
                lines: vec![
                    "Kast Alaska".into(),
                    "Inhoud per pak voor".into(),
                    "1,62 m²".into(),
                    "135,20".into(),
                    "Tegel: Mat".into(),
                ],
                headings: vec![0],
                labelled: vec![],
                images: vec![],
            }
        );
        assert_eq!(
            page.value("inhoud per pak voor"),
            Some(("1,62 m²", "Inhoud per pak voor 1,62 m²".to_owned()))
        );
        assert_eq!(page.value("Tegel"), Some(("Mat", "Tegel: Mat".to_owned())));
        assert_eq!(page.value("Kleur"), None);
    }

    /// The old price is struck out; the label of the current one counts.
    #[test]
    fn a_struck_out_amount_is_not_the_price() {
        let page = Page::read(
            r#"<div><del aria-label="EUR 21.99"><p>21,</p></del><ins aria-label="EUR 17.59"><p>17,</p></ins><p>per m²</p><a aria-label="Gemiddelde beoordeling: 3.4">x</a></div>"#,
        );
        assert_eq!(
            page.labelled,
            [Labelled {
                line: 1,
                label: "EUR 17.59".into()
            }]
        );
        assert_eq!(page.lines, ["21,", "17,", "per m²", "x"]);
    }

    #[test]
    fn images_are_found_by_their_description() {
        let page = Page::read(
            r#"<img itemprop="image" src="https://a/logo.png" alt=""/><img alt="Kast  Alaska" src="https://a/1.webp"><img src='https://a/2.webp' alt="a > b"><img alt="Kast Alaska" src="https://a/3.webp">"#,
        );
        assert_eq!(
            page.images,
            [
                ("Kast Alaska".to_owned(), "https://a/1.webp".to_owned()),
                ("a > b".to_owned(), "https://a/2.webp".to_owned()),
                ("Kast Alaska".to_owned(), "https://a/3.webp".to_owned()),
            ]
        );
        assert_eq!(page.image("Kast Alaska"), Some("https://a/1.webp"));
        assert_eq!(page.image("Kast"), None);
    }

    /// A specification table's line wins over a sentence that starts with
    /// the same word.
    #[test]
    fn a_table_value_wins_over_a_sentence() {
        let page =
            Page::read("<p>Inhoud: 2,5 L.</p><dt>Inhoud</dt><dd>2.5 l</dd><p>Kleur: wit</p>");
        assert_eq!(
            page.value("Inhoud"),
            Some(("2.5 l", "Inhoud 2.5 l".to_owned()))
        );
        assert_eq!(page.value("Kleur"), Some(("wit", "Kleur: wit".to_owned())));
    }

    #[test]
    fn entities_are_read() {
        assert_eq!(
            decode("a &amp; b &#8364; &#x20AC; &bogus; &"),
            "a & b € € &bogus; &"
        );
    }
}
