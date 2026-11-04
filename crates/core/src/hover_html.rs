//! Renders a language server's Markdown (hover, documentation) to HTML the
//! view can insert as is (`docs/ARCHITECTURE.md`, 4.7):
//!
//! - every piece of text is HTML-escaped, including raw HTML in the Markdown;
//! - images become their alt text;
//! - links keep only `http`, `https` and `mailto` URLs (others render as
//!   plain text);
//! - fenced code is highlighted with our tree-sitter languages, using the
//!   editor's `tok-*` classes.

use std::fmt::Write as _;

use ferrule_buffer::Rope;
use ferrule_syntax::{LanguageRegistry, SyntaxTree};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::convert::token_to_ipc;

/// Renders Markdown to sanitized HTML.
pub(crate) fn render_markdown(markdown: &str, registry: &LanguageRegistry) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut html = String::new();
    // Per open link: whether it was rendered as an <a>.
    let mut links: Vec<bool> = Vec::new();
    let mut image_depth = 0_usize;
    let mut code: Option<(String, String)> = None;
    for event in Parser::new_ext(markdown, options) {
        if let Some((_, buffer)) = &mut code {
            match event {
                Event::Text(text) => buffer.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((lang, text)) = code.take() {
                        html.push_str(&code_block(&lang, &text, registry));
                    }
                }
                _ => {}
            }
            continue;
        }
        if image_depth > 0 {
            // Inside an image only the alt text is kept.
            match event {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                Event::Text(text)
                | Event::Code(text)
                | Event::Html(text)
                | Event::InlineHtml(text) => escape_into(&mut html, &text),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => html.push_str("<p>"),
                Tag::Heading { level, .. } => {
                    let _ = write!(html, "<h{}>", heading(level));
                }
                Tag::BlockQuote(_) => html.push_str("<blockquote>"),
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split_whitespace().next().unwrap_or("").to_owned()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    code = Some((lang, String::new()));
                }
                Tag::List(Some(start)) => {
                    let _ = write!(html, "<ol start=\"{start}\">");
                }
                Tag::List(None) => html.push_str("<ul>"),
                Tag::Item => html.push_str("<li>"),
                Tag::Emphasis => html.push_str("<em>"),
                Tag::Strong => html.push_str("<strong>"),
                Tag::Strikethrough => html.push_str("<del>"),
                Tag::Link {
                    dest_url, title, ..
                } => {
                    let safe = is_safe_url(&dest_url);
                    if safe {
                        html.push_str("<a href=\"");
                        escape_into(&mut html, &dest_url);
                        html.push('"');
                        if !title.is_empty() {
                            html.push_str(" title=\"");
                            escape_into(&mut html, &title);
                            html.push('"');
                        }
                        html.push_str(" rel=\"noreferrer noopener\" target=\"_blank\">");
                    }
                    links.push(safe);
                }
                Tag::Image { .. } => image_depth = 1,
                Tag::Table(_) => html.push_str("<table>"),
                Tag::TableHead => html.push_str("<thead><tr>"),
                Tag::TableRow => html.push_str("<tr>"),
                Tag::TableCell => html.push_str("<td>"),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => html.push_str("</p>"),
                TagEnd::Heading(level) => {
                    let _ = write!(html, "</h{}>", heading(level));
                }
                TagEnd::BlockQuote(_) => html.push_str("</blockquote>"),
                TagEnd::List(true) => html.push_str("</ol>"),
                TagEnd::List(false) => html.push_str("</ul>"),
                TagEnd::Item => html.push_str("</li>"),
                TagEnd::Emphasis => html.push_str("</em>"),
                TagEnd::Strong => html.push_str("</strong>"),
                TagEnd::Strikethrough => html.push_str("</del>"),
                TagEnd::Link => {
                    if links.pop() == Some(true) {
                        html.push_str("</a>");
                    }
                }
                TagEnd::Table => html.push_str("</table>"),
                TagEnd::TableHead => html.push_str("</tr></thead>"),
                TagEnd::TableRow => html.push_str("</tr>"),
                TagEnd::TableCell => html.push_str("</td>"),
                _ => {}
            },
            // Raw HTML is shown as text, never interpreted.
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                escape_into(&mut html, &text);
            }
            Event::Code(text) => {
                html.push_str("<code>");
                escape_into(&mut html, &text);
                html.push_str("</code>");
            }
            Event::SoftBreak => html.push('\n'),
            Event::HardBreak => html.push_str("<br>"),
            Event::Rule => html.push_str("<hr>"),
            Event::TaskListMarker(done) => html.push_str(if done { "[x] " } else { "[ ] " }),
            Event::FootnoteReference(name) => {
                html.push('[');
                escape_into(&mut html, &name);
                html.push(']');
            }
            _ => {}
        }
    }
    if let Some((lang, text)) = code.take() {
        html.push_str(&code_block(&lang, &text, registry));
    }
    html
}

/// Plain text as HTML (escaped, line breaks kept by the view's CSS).
pub(crate) fn plain_text_html(text: &str) -> String {
    let mut html = String::from("<p class=\"hover-plain\">");
    escape_into(&mut html, text);
    html.push_str("</p>");
    html
}

fn heading(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn is_safe_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

fn escape_into(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

/// A code block, highlighted when the fence names a known language.
fn code_block(lang: &str, text: &str, registry: &LanguageRegistry) -> String {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut html = String::from("<pre class=\"hover-code\"><code>");
    let spans = registry
        .for_name(lang)
        .and_then(|language| SyntaxTree::new(language).ok())
        .map(|tree| {
            let mut tree = tree;
            tree.highlight(&Rope::from_str(text), 0..text.len())
        })
        .unwrap_or_default();
    let mut at = 0;
    for span in spans {
        if span.start < at || span.end > text.len() || span.start >= span.end {
            continue;
        }
        escape_into(&mut html, &text[at..span.start]);
        let _ = write!(html, "<span class=\"tok-{}\">", class_name(span.class));
        escape_into(&mut html, &text[span.start..span.end]);
        html.push_str("</span>");
        at = span.end;
    }
    escape_into(&mut html, &text[at..]);
    html.push_str("</code></pre>");
    html
}

/// The token class as the view names it (`tok-<camelCase>`).
fn class_name(class: ferrule_syntax::TokenClass) -> String {
    serde_json::to_value(token_to_ipc(class))
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> LanguageRegistry {
        let mut registry = LanguageRegistry::default();
        for def in ferrule_lang_js::languages() {
            registry.register(def).unwrap();
        }
        registry
    }

    #[test]
    fn renders_typescript_hover_with_highlighted_code() {
        let html = render_markdown(
            "```typescript\nfunction greet(name: string): string\n```\nGreets *someone*.",
            &registry(),
        );
        assert!(
            html.starts_with("<pre class=\"hover-code\"><code>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"tok-keyword\">function</span>"),
            "{html}"
        );
        assert!(html.contains("<p>Greets <em>someone</em>.</p>"), "{html}");
    }

    #[test]
    fn escapes_raw_html_and_drops_unsafe_links_and_images() {
        let html = render_markdown(
            "<script>alert(1)</script>\n\nx <b onclick=\"y\">b</b> \
             [ok](https://example.com \"t\") [bad](javascript:alert(1)) \
             ![alt <i>](https://example.com/a.png)",
            &registry(),
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<b "), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(
            html.contains("<a href=\"https://example.com\" title=\"t\""),
            "{html}"
        );
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("bad"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("alt &lt;i&gt;"), "{html}");
    }

    #[test]
    fn unknown_fence_languages_are_escaped_plain_code() {
        let html = render_markdown("```nope\na < b\n```", &registry());
        assert_eq!(
            html,
            "<pre class=\"hover-code\"><code>a &lt; b</code></pre>"
        );
        assert_eq!(
            plain_text_html("<x>"),
            "<p class=\"hover-plain\">&lt;x&gt;</p>"
        );
    }
}
