use crate::{FeatureView, Graph};
use std::fmt::Write as _;

fn text(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '\0' => out.push('\u{fffd}'),
            '"' => out.push_str("&#34;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '+' => out.push_str("&#43;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn fragment(out: &mut String, value: &str) {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02x}");
        }
    }
    text(out, &encoded);
}

#[must_use]
pub fn render(_graph: &Graph, views: &[FeatureView]) -> String {
    let mut o = String::new();
    o.push_str(include_str!("head.html"));
    for v in views {
        let f = &v.feature;
        o.push_str("<div class=\"f\" id=\"");
        text(&mut o, &f.id);
        o.push_str("\">\n<h2>");
        text(&mut o, &f.id);
        o.push_str("<span class=\"st st-");
        text(&mut o, &f.status);
        o.push_str("\">");
        text(&mut o, &f.status);
        o.push_str("</span></h2>\n<div>");
        text(&mut o, &f.title);
        o.push_str("</div>\n");
        if !f.notes.is_empty() {
            o.push_str("<div>");
            text(&mut o, &f.notes);
            o.push_str("</div>");
        }
        o.push_str("\n<div class=\"meta\">");
        text(&mut o, &f.milestone);
        o.push_str(" · requirements: ");
        for r in &f.requirements {
            text(&mut o, r);
            o.push(' ');
        }
        o.push_str("· modules: ");
        for m in &f.modules {
            text(&mut o, m);
            o.push(' ');
        }
        if !f.depends_on.is_empty() {
            o.push_str("· depends on: ");
            for d in &f.depends_on {
                o.push_str("<a href=\"#");
                fragment(&mut o, d);
                o.push_str("\">");
                text(&mut o, d);
                o.push_str("</a> ");
            }
        }
        o.push_str("</div>\n<table><tr><th>scenario</th><th>file</th><th>levels</th></tr>\n");
        for s in &v.views {
            o.push_str("<tr><td>");
            text(&mut o, &s.id);
            if !s.reason.is_empty() {
                o.push_str(" — ");
                text(&mut o, &s.reason);
            }
            o.push_str("</td><td>");
            if s.file.is_empty() {
                o.push_str("<span class=\"missing\">not written</span>");
            } else {
                text(&mut o, &s.file);
            }
            o.push_str("</td>\n<td>");
            for r in &s.results {
                o.push_str("<span class=\"");
                text(&mut o, &r.state);
                o.push_str("\">");
                text(&mut o, &r.level);
                o.push(':');
                text(&mut o, &r.state);
                o.push_str("</span> ");
            }
            o.push_str("</td></tr>");
        }
        o.push_str("\n</table></div>");
    }
    o.push_str("\n</body></html>");
    o
}
