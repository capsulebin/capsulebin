//! Markdown to HTML for the operator's legal texts.
//!
//! The texts come from the operator, not from the public, but they still end
//! up on every visitor's screen under our origin, so the output is kept to
//! what a legal text needs: no raw HTML (it is shown as text), no images (they
//! become links, as in the web client), and links only to `http(s)`, `mailto`,
//! `tel`, paths on this host and fragments. A link to anything else keeps its
//! text and loses the link.

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

pub fn render(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;

    // For every open link or image: whether its start tag was kept, so that
    // the matching end tag is kept or dropped with it.
    let mut open: Vec<bool> = Vec::new();

    let events = Parser::new_ext(markdown, options).filter_map(|event| match event {
        // Raw HTML is shown, not interpreted.
        Event::Html(html) | Event::InlineHtml(html) => Some(Event::Text(html)),
        Event::Start(Tag::HtmlBlock) => Some(Event::Start(Tag::Paragraph)),
        Event::End(TagEnd::HtmlBlock) => Some(Event::End(TagEnd::Paragraph)),

        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let keep = is_safe_link(&dest_url);
            open.push(keep);
            keep.then_some(Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }))
        }
        Event::End(TagEnd::Link) => open.pop().unwrap_or(false).then_some(Event::End(TagEnd::Link)),

        // An image would load from wherever its URL points; it becomes a link
        // to it instead, labelled with its alt text.
        Event::Start(Tag::Image { dest_url, title, id, .. }) => {
            let keep = is_safe_link(&dest_url);
            open.push(keep);
            keep.then_some(Event::Start(Tag::Link {
                link_type: LinkType::Inline,
                dest_url,
                title,
                id,
            }))
        }
        Event::End(TagEnd::Image) => open.pop().unwrap_or(false).then_some(Event::End(TagEnd::Link)),

        other => Some(other),
    });

    let mut html = String::with_capacity(markdown.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html, events);
    html
}

fn is_safe_link(url: &CowStr<'_>) -> bool {
    let url: &str = url;
    if url.contains('\\') || url.chars().any(char::is_control) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    ["https://", "http://", "mailto:", "tel:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
        || (url.starts_with('/') && !url.starts_with("//"))
        || url.starts_with('#')
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn renders_what_a_legal_text_needs() {
        let html = render(
            "Operator **GmbH**\n\nStreet 1  \n12345 City\n\n## Contact\n\n- [Mail](mailto:a@b.example)\n- [Phone](tel:+49123)\n- [Web](https://operator.example)\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );

        assert!(html.contains("<strong>GmbH</strong>"));
        assert!(html.contains("Street 1<br />"));
        assert!(html.contains("<h2>Contact</h2>"));
        assert!(html.contains(r#"<a href="mailto:a@b.example">Mail</a>"#));
        assert!(html.contains(r#"<a href="tel:+49123">Phone</a>"#));
        assert!(html.contains(r#"<a href="https://operator.example">Web</a>"#));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn raw_html_is_shown_as_text() {
        let html = render("<script>alert(1)</script>\n\nText with <img src=x onerror=alert(1)> inline.\n");

        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"), "{html}");
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"), "{html}");
    }

    #[test]
    fn unsafe_links_keep_their_text_and_lose_the_link() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:x",
            "//evil.example",
            "/\\evil.example",
            "relative/path",
        ] {
            let html = render(&format!("[click]({url}) and <{url}>"));
            assert!(!html.contains("<a "), "{url}: {html}");
            assert!(html.contains("click"), "{url}: {html}");
        }
    }

    #[test]
    fn images_become_links_and_never_load() {
        let html = render("![Logo](https://operator.example/logo.png) ![x](javascript:alert(1))");

        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains(r#"<a href="https://operator.example/logo.png">Logo</a>"#), "{html}");
        assert_eq!(html.matches("<a ").count(), 1, "{html}");
    }

    #[test]
    fn nested_links_stay_balanced() {
        let html = render("[![a](javascript:x)](https://ok.example) [b](javascript:y) [c](/terms)");

        assert_eq!(html.matches("<a ").count(), html.matches("</a>").count(), "{html}");
        assert!(html.contains(r#"<a href="/terms">c</a>"#), "{html}");
    }

    #[test]
    fn attribute_values_are_escaped() {
        let html = render(r#"[x](https://ok.example/?a=1&b="2" "t\"itle")"#);
        assert!(!html.contains(r#"b="2""#), "{html}");
    }
}
