#![forbid(unsafe_code)]

pub const STYLESHEET: &str = include_str!("../assets/pages.css");
pub const STYLESHEET_PATH: &str = "/assets/pages.css";

use std::fmt::Write;

const MARK: &str = r#"<svg class="mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M2 18h20M4 18V14a8 8 0 0 1 16 0v4M9 18v-5.2M15 18v-5.2M12 18V10" /></svg>"#;

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            '+' => out.push_str("&#43;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Page<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub scripts: &'a [&'a str],
    pub nav: &'a str,
    pub wide: bool,
}

impl Page<'_> {
    pub fn render(&self) -> String {
        let mut scripts = String::new();
        for path in self.scripts {
            let _ = write!(scripts, "<script src=\"{}\" defer></script>", escape(path));
        }
        let title = if self.title.is_empty() {
            "Apple Bridge".to_owned()
        } else {
            format!("{} · Apple Bridge", escape(self.title))
        };
        let width = if self.wide { " wide" } else { "" };
        format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
             <meta name=\"robots\" content=\"noindex\"><title>{title}</title>\
             <link rel=\"stylesheet\" href=\"{STYLESHEET_PATH}\">{scripts}</head>\
             <body><div class=\"shell{width}\"><header class=\"brand\">\
             <span class=\"wordmark\">{MARK}Apple Bridge</span><nav>{nav}</nav></header>\
             <main class=\"card\">{body}</main></div></body></html>",
            nav = self.nav,
            body = self.body,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_attribute_and_element_contexts() {
        assert_eq!(
            escape("<a href=\"x\">'&+"),
            "&lt;a href=&#34;x&#34;&gt;&#39;&amp;&#43;"
        );
    }

    #[test]
    fn page_links_only_same_origin_assets_and_has_no_inline_style() {
        let html = Page {
            title: "Approve",
            body: "<h1>Hello</h1>",
            scripts: &["/assets/connector.js"],
            ..Page::default()
        }
        .render();
        assert!(html.contains("<title>Approve · Apple Bridge</title>"));
        assert!(html.contains("src=\"/assets/connector.js\""));
        assert!(html.contains(STYLESHEET_PATH));
        assert!(!html.contains("style="));
        assert!(!html.contains("<style"));
        assert!(!html.contains("http://") && !html.contains("https://"));
    }

    #[test]
    fn stylesheet_keeps_the_agreed_look() {
        assert!(!STYLESHEET.contains("monospace"), "no monospace labels");
        assert!(!STYLESHEET.contains("999px"), "no pill shapes");
        assert!(!STYLESHEET.contains("@import"), "no remote fonts");
    }
}
