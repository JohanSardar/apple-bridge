use ab_pages::{Page, escape};
use ab_trust::{Grant, oauth::Client};
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::Value;
use std::fmt::Write;

const SCRIPTS: &[&str] = &["/webauthn.js"];
const CONNECTOR_SCRIPTS: &[&str] = &["/webauthn.js", "/assets/connector.js"];
const SIGN_IN_ROW: &str = "<button id=\"signin\" class=\"primary full\" type=\"button\">Sign in with passkey</button><p id=\"message\" class=\"status small\" role=\"status\"></p>";

pub(crate) fn document(status: StatusCode, page: Page<'_>) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        page.render(),
    )
        .into_response()
}

fn plain(title: &str, body: &str) -> Response {
    document(
        StatusCode::OK,
        Page {
            title,
            body,
            scripts: SCRIPTS,
            ..Page::default()
        },
    )
}

pub(crate) fn home() -> Response {
    plain(
        "Sign in",
        &format!(
            "<h1>Apple Bridge is running on this Mac</h1><p class=\"lede\">Sign in to review approvals. Apps connect from their own settings using your connector address.</p><div class=\"section\">{SIGN_IN_ROW}</div>"
        ),
    )
}

pub(crate) fn enroll() -> Response {
    plain(
        "Add a passkey",
        &format!(
            "<h1>Add your first passkey</h1><p class=\"lede\">It lets you approve from this browser when your Mac and iPhone are out of reach.</p><div class=\"section\">{SIGN_IN_ROW}</div>"
        ),
    )
}

pub(crate) fn sign_in_for_approvals() -> Response {
    plain(
        "Approvals",
        &format!(
            "<h1>Sign in to see approvals</h1><p class=\"lede\">Use the passkey you set up for this Mac.</p><div class=\"section\">{SIGN_IN_ROW}</div>"
        ),
    )
}

const SOURCES: &[(&str, &str)] = &[
    ("messages", "Messages"),
    ("mail", "Mail"),
    ("calendar", "Calendar"),
    ("contacts", "Contacts"),
    ("notes", "Notes"),
    ("reminders", "Reminders"),
    ("safari", "Safari"),
    ("files", "Files"),
    ("phone", "Phone"),
    ("voicemail", "Voicemail"),
    ("voice_memos", "Voice Memos"),
    ("health", "Health"),
    ("location", "Location"),
    ("maps", "Maps"),
    ("usage", "Screen Time"),
    ("focus", "Focus"),
    ("shortcuts", "Shortcuts"),
    ("iphone", "iPhone"),
    ("mac", "Mac"),
    ("audio", "Audio"),
    ("capture", "Capture"),
    ("pdf", "PDF"),
    ("print", "Print"),
    ("speech", "Speech"),
    ("vision", "Vision"),
];

fn ranked(sources: &mut Vec<&str>) -> Vec<String> {
    let rank = |source: &str| {
        SOURCES
            .iter()
            .position(|(id, _)| *id == source)
            .unwrap_or(SOURCES.len())
    };
    sources.sort_by_key(|source| (rank(source), *source));
    sources.dedup();
    sources
        .iter()
        .map(|source| {
            SOURCES
                .iter()
                .find(|(id, _)| id == source)
                .map_or_else(|| (*source).to_owned(), |(_, title)| (*title).to_owned())
        })
        .collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Access {
    pub read: Vec<String>,
    pub change: Vec<String>,
}

pub(crate) fn access(scopes: &[Grant]) -> Access {
    let mut read = Vec::new();
    let mut change = Vec::new();
    for grant in scopes {
        if let Some((source, action)) = grant.as_str().split_once(':') {
            read.push(source);
            if action != "read" {
                change.push(source);
            }
        }
    }
    Access {
        read: ranked(&mut read),
        change: ranked(&mut change),
    }
}

pub(crate) fn access_sentence(scopes: &[Grant]) -> String {
    let access = access(scopes);
    let mut parts = Vec::new();
    if !access.read.is_empty() {
        parts.push(format!("Read: {}", access.read.join(", ")));
    }
    if !access.change.is_empty() {
        parts.push(format!(
            "Ask to change (each change needs your approval): {}",
            access.change.join(", ")
        ));
    }
    parts.join("\n")
}

fn list(names: &[String]) -> String {
    const SHOWN: usize = 7;
    if names.len() <= SHOWN {
        return format!("<p>{}</p>", escape(&names.join(", ")));
    }
    let (first, rest) = names.split_at(SHOWN);
    format!(
        "<p>{}</p><details class=\"small\"><summary>and {} more</summary><p>{}</p></details>",
        escape(&first.join(", ")),
        rest.len(),
        escape(&rest.join(", "))
    )
}

fn access_section(scopes: &[Grant]) -> String {
    let access = access(scopes);
    let mut body = String::from("<section class=\"section access\">");
    if access.read == access.change {
        let _ = write!(
            body,
            "<div><h2>Can read, and ask to make changes</h2>{}<p class=\"small\">Nothing changes until you approve that change.</p></div></section>",
            list(&access.read)
        );
        return body;
    }
    if !access.read.is_empty() {
        let _ = write!(body, "<div><h2>Can read</h2>{}</div>", list(&access.read));
    }
    if !access.change.is_empty() {
        let _ = write!(
            body,
            "<div><h2>Can ask to make changes</h2>{}<p class=\"small\">Nothing changes until you approve that change.</p></div>",
            list(&access.change)
        );
    }
    body.push_str("</section>");
    body
}

fn request_header(client: &Client, redirect_host: &str) -> String {
    let name = client.client_name.as_str();
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string());
    format!(
        "<header class=\"request\"><div class=\"tile\" aria-hidden=\"true\">{}</div><div><h1>{} wants to connect to your Mac</h1><p class=\"lede\">{}</p></div></header>",
        escape(&initial),
        escape(name),
        escape(&ab_trust::oauth::callback_label(redirect_host))
    )
}

pub(crate) struct Consent<'a> {
    pub client: &'a Client,
    pub redirect_host: &'a str,
    pub scopes: &'a [Grant],
    pub cancel: &'a str,
    pub signed_in: bool,
    pub passkey_available: bool,
}

pub(crate) fn consent(c: &Consent<'_>) -> Response {
    let mut body = request_header(c.client, c.redirect_host);
    body.push_str(&access_section(c.scopes));
    if c.signed_in {
        body.push_str("<form method=\"post\" class=\"section\"><fieldset class=\"stack\"><div><label class=\"choice\"><input type=\"radio\" name=\"level\" value=\"approve\" checked><span><strong>Read, and ask me before any change</strong><span class=\"muted small\">Recommended</span></span></label><label class=\"choice\"><input type=\"radio\" name=\"level\" value=\"read\"><span><strong>Read only</strong><span class=\"muted small\">Change requests are refused</span></span></label></div></fieldset>");
        for grant in c.scopes {
            let _ = write!(
                body,
                "<input type=\"hidden\" name=\"scopes\" value=\"{}\">",
                escape(grant.as_str())
            );
        }
        body.push_str("<div class=\"actions end\"><button name=\"decision\" value=\"reject\">Deny</button><button name=\"decision\" value=\"approve\" class=\"primary\">Allow</button></div></form>");
    } else {
        let _ = write!(
            body,
            "<section id=\"approval\" class=\"confirm\"><h2>Approve on your Mac or iPhone</h2><p class=\"muted small\">Allow it there only if the code matches.</p><p id=\"approval-code\" class=\"code\" aria-label=\"Matching code\">····</p><p id=\"approval-status\" class=\"status\" role=\"status\">Asking your Mac…</p></section><div class=\"actions\"><a href=\"{}\">Cancel</a>",
            escape(c.cancel)
        );
        if c.passkey_available {
            body.push_str("<button id=\"use-passkey\" class=\"link\" type=\"button\">Use a passkey instead</button>");
        }
        body.push_str("</div>");
        if c.passkey_available {
            let _ = write!(
                body,
                "<section id=\"passkey\" class=\"section\" hidden>{SIGN_IN_ROW}</section>"
            );
        }
    }
    document(
        StatusCode::OK,
        Page {
            title: "Approve connection",
            body: &body,
            scripts: if c.signed_in {
                SCRIPTS
            } else {
                CONNECTOR_SCRIPTS
            },
            ..Page::default()
        },
    )
}

pub(crate) fn invalid_request() -> Response {
    document(
        StatusCode::BAD_REQUEST,
        Page {
            title: "Can't connect",
            body: "<h1>This connection request isn't valid</h1><p class=\"lede\">Nothing was shared. Start again from the app you were connecting.</p>",
            ..Page::default()
        },
    )
}

pub(crate) fn connector_redirect(destination: &str) -> Response {
    plain(
        "Connected",
        &format!(
            "<h1>Decision recorded</h1><p class=\"lede\">Returning to the app.</p><div class=\"actions\"><a id=\"connector-redirect\" href=\"{}\">Continue</a></div>",
            escape(destination)
        ),
    )
}

const UNPREVIEWABLE_CONSENT: &str = "<label class=\"choice\"><input type=\"checkbox\" name=\"accept_unpreviewable\" value=\"1\" required><span><strong>Send it anyway</strong><span class=\"muted small\">I can't see inside at least one attachment and accept sending these exact bytes.</span></span></label>";

fn attachment_rows(id: &str, row: &Value) -> String {
    let mut rows = String::new();
    if row["attachment_error"] == true {
        rows.push_str("<p class=\"small\">An attachment can't be read right now, so this can't be approved. You can reject it.</p>");
    }
    for attachment in row["attachments"].as_array().into_iter().flatten() {
        let index = attachment["index"].as_u64().unwrap_or_default();
        let preview = attachment["preview"].as_str().unwrap_or_default();
        let (action, note) = match preview {
            "text" | "image" => ("Preview", ""),
            "too_large" => (
                "",
                "<p class=\"small\">This file can't be previewed: it is too large to inspect here.</p>",
            ),
            _ => (
                "Download",
                "<p class=\"small\">This file can't be previewed here. Download it to inspect it somewhere safe.</p>",
            ),
        };
        let link = if action.is_empty() {
            String::new()
        } else {
            format!("<a href=\"/approvals/{id}/attachments/{index}\">{action}</a>")
        };
        let _ = write!(
            rows,
            "<section class=\"section\"><h3>Attachment {}: {}</h3><p class=\"small\">Detected {}, {} bytes</p><p class=\"small\">SHA-256 {}</p>{note}{link}</section>",
            index + 1,
            escape(attachment["name"].as_str().unwrap_or_default()),
            escape(attachment["media_type"].as_str().unwrap_or_default()),
            attachment["size"].as_u64().unwrap_or_default(),
            escape(attachment["digest"].as_str().unwrap_or_default()),
        );
    }
    rows
}

pub(crate) fn attachment_file(index: usize, file: crate::web::AttachmentFile) -> Response {
    const IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
    let (media_type, disposition) = match file.preview.as_str() {
        "text" => (
            "text/plain; charset=utf-8".to_owned(),
            "inline".to_owned(),
        ),
        "image" if IMAGES.contains(&file.media_type.as_str()) => {
            (file.media_type, "inline".to_owned())
        }
        _ => (
            "application/octet-stream".to_owned(),
            format!("attachment; filename=\"attachment-{}.bin\"", index + 1),
        ),
    };
    (
        [
            (header::CONTENT_TYPE, media_type),
            (header::CONTENT_DISPOSITION, disposition),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox".to_owned(),
            ),
        ],
        file.bytes,
    )
        .into_response()
}

pub(crate) fn approvals(rows: &[Value]) -> Response {
    let mut body = String::from("<h1>Pending approvals</h1>");
    if rows.is_empty() {
        body.push_str("<p class=\"lede\">Nothing is waiting for you.</p>");
    }
    for row in rows {
        let id = escape(row["command_id"].as_str().unwrap_or_default());
        let unreadable = row["attachment_error"] == true;
        let approve = if unreadable {
            ""
        } else {
            "<button name=\"decision\" value=\"approve\" class=\"primary\">Approve</button>"
        };
        let _ = write!(
            body,
            "<article class=\"item\"><div class=\"item-head\"><h2>{}</h2><span class=\"tag\">{}</span></div><pre class=\"data\">{}</pre>{}<details><summary class=\"small\">Technical details</summary><pre class=\"data\">{}</pre></details><form method=\"post\" action=\"/approvals/{id}\"><input type=\"hidden\" name=\"attachments\" value=\"{}\">{}<div class=\"actions end\"><button name=\"decision\" value=\"reject\" formnovalidate>Reject</button>{approve}</div></form></article>",
            escape(row["operation"].as_str().unwrap_or_default()),
            escape(row["principal"].as_str().unwrap_or_default()),
            escape(row["owner_arguments"].as_str().unwrap_or_default()),
            attachment_rows(&id, row),
            escape(row["arguments"].as_str().unwrap_or_default()),
            escape(row["binding"].as_str().unwrap_or_default()),
            if row["requires_consent"] == true { UNPREVIEWABLE_CONSENT } else { "" },
        );
    }
    document(
        StatusCode::OK,
        Page {
            title: "Approvals",
            body: &body,
            scripts: SCRIPTS,
            nav: "<a href=\"/approvals\">Approvals</a>",
            wide: true,
        },
    )
}

pub(crate) fn command_status(record: &Value) -> Response {
    let body = format!(
        "<h1>Command status</h1><pre class=\"data\">{}</pre><div class=\"actions\"><a href=\"/approvals\">Back to approvals</a></div>",
        escape(&serde_json::to_string_pretty(record).unwrap_or_default())
    );
    document(
        StatusCode::OK,
        Page {
            title: "Command status",
            body: &body,
            scripts: SCRIPTS,
            nav: "<a href=\"/approvals\">Approvals</a>",
            wide: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_grant_also_shows_that_it_reads() {
        let grants = [Grant::parse("messages:write").expect("grant")];
        assert_eq!(
            access(&grants),
            Access {
                read: vec!["Messages".into()],
                change: vec!["Messages".into()],
            }
        );
        assert_eq!(
            access_sentence(&grants),
            "Read: Messages\nAsk to change (each change needs your approval): Messages"
        );
        let section = access_section(&grants);
        assert!(section.contains("Can read, and ask to make changes"), "{section}");
    }

    #[test]
    fn scopes_become_product_words() {
        let grants: Vec<Grant> = ["messages:read", "calendar:read", "calendar:write"]
            .iter()
            .filter_map(|s| Grant::parse(s).ok())
            .collect();
        assert_eq!(
            access(&grants),
            Access {
                read: vec!["Messages".into(), "Calendar".into()],
                change: vec!["Calendar".into()],
            }
        );
        assert_eq!(
            access_sentence(&grants),
            "Read: Messages, Calendar\nAsk to change (each change needs your approval): Calendar"
        );
    }
}
