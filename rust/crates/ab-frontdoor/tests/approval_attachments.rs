#![allow(clippy::unwrap_used)]
use ab_frontdoor::{
    web::{Approvals, AttachmentFile, Reviewed, Web},
    web_connector::Connectors,
};
use ab_trust::{Authority, Error, Principal};
use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tower::ServiceExt;

const ORIGIN: &str = "http://127.0.0.1:7780";
const HOST: &str = "127.0.0.1:7780";
const DIGEST: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

#[derive(Debug, Default)]
struct Fake {
    decided: Mutex<Vec<(String, bool, String, bool)>>,
}
impl Approvals for Fake {
    fn pending(&self) -> Result<Vec<Value>, Error> {
        Ok(vec![
            json!({"command_id":"cmd_text","operation":"messages.send","principal":"agent:x","arguments":"{}","owner_arguments":"{}",
                "attachments":[{"index":0,"name":"<b>harmless</b>.pdf","digest":DIGEST,"size":12,"media_type":"text/plain","preview":"text"}],
                "binding":"binding-text","requires_consent":false}),
            json!({"command_id":"cmd_binary","operation":"messages.send","principal":"agent:x","arguments":"{}","owner_arguments":"{}",
                "attachments":[{"index":0,"name":"a.bin","digest":DIGEST,"size":9,"media_type":"application/octet-stream","preview":"download_only"}],
                "binding":"binding-binary","requires_consent":true}),
            json!({"command_id":"cmd_plain","operation":"reminders.create","principal":"agent:x","arguments":"{}","owner_arguments":"{}"}),
        ])
    }
    fn command(&self, _: &str) -> Result<Value, Error> {
        Ok(json!({"state":"awaiting_approval"}))
    }
    fn attachment(&self, id: &str, index: usize) -> Result<AttachmentFile, Error> {
        if index != 0 {
            return Err(Error::Invalid);
        }
        Ok(match id {
            "cmd_text" => AttachmentFile {
                media_type: "text/plain".into(),
                preview: "text".into(),
                bytes: b"<script>alert(1)</script>".to_vec(),
            },
            "cmd_image" => AttachmentFile {
                media_type: "image/png".into(),
                preview: "image".into(),
                bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
            },
            _ => AttachmentFile {
                media_type: "application/pdf".into(),
                preview: "download_only".into(),
                bytes: b"%PDF-synthetic".to_vec(),
            },
        })
    }
    fn decide(
        &self,
        _: &Principal,
        id: &str,
        approve: bool,
        reviewed: &Reviewed<'_>,
    ) -> Result<Value, Error> {
        self.decided.lock().unwrap().push((
            id.into(),
            approve,
            reviewed.binding.into(),
            reviewed.accepted_unpreviewable,
        ));
        Ok(json!({"state":"admitted"}))
    }
}

struct Server {
    _home: tempfile::TempDir,
    router: axum::Router,
    cookie: String,
    fake: Arc<Fake>,
}
fn server() -> Server {
    let home = tempfile::tempdir().unwrap();
    let authority = Arc::new(Authority::open(home.path()).unwrap());
    let address: SocketAddr = HOST.parse().unwrap();
    let (token, _) = authority.create_device_session("trusted").unwrap();
    let connectors = Connectors::new(authority.clone());
    let fake = Arc::new(Fake::default());
    let web = Web::new(
        authority,
        address,
        String::new(),
        Some(fake.clone()),
        connectors,
    );
    Server {
        _home: home,
        router: web.router(),
        cookie: format!("ab_session_local={token}"),
        fake,
    }
}
impl Server {
    async fn get(&self, path: &str) -> Response<Body> {
        self.router
            .clone()
            .oneshot(
                Request::get(path)
                    .header(header::HOST, HOST)
                    .header(header::COOKIE, &self.cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn post(&self, path: &str, body: &str) -> Response<Body> {
        self.router
            .clone()
            .oneshot(
                Request::post(path)
                    .header(header::HOST, HOST)
                    .header(header::ORIGIN, ORIGIN)
                    .header(header::COOKIE, &self.cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}
async fn text(response: Response<Body>) -> String {
    String::from_utf8(to_bytes(response.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap()
}
fn header_of<'a>(response: &'a Response<Body>, name: header::HeaderName) -> &'a str {
    response.headers()[name].to_str().unwrap()
}

#[tokio::test]
async fn the_approval_page_shows_what_each_attachment_is() {
    let s = server();
    let page = text(s.get("/approvals").await).await;
    assert!(page.contains("&lt;b&gt;harmless&lt;/b&gt;.pdf"), "{page}");
    assert!(!page.contains("<b>harmless"));
    assert!(page.contains("text/plain"));
    assert!(page.contains(DIGEST));
    assert!(page.contains("12 bytes"));
    assert!(page.contains("href=\"/approvals/cmd_text/attachments/0\""));
    assert!(page.contains("name=\"attachments\" value=\"binding-text\""));
    assert!(page.contains("name=\"attachments\" value=\"binding-binary\""));
}

#[tokio::test]
async fn an_unpreviewable_attachment_says_so_and_asks_for_consent() {
    let s = server();
    let page = text(s.get("/approvals").await).await;
    let binary = page.split("cmd_binary").nth(1).unwrap();
    assert!(page.contains("can't be previewed"), "{page}");
    assert!(binary.contains("name=\"accept_unpreviewable\" value=\"1\" required"));
    let plain = &page[page.find("/approvals/cmd_plain").unwrap()..];
    assert!(!plain.contains("accept_unpreviewable"));
    let text_row = &page[..page.find("/approvals/cmd_binary").unwrap()];
    assert!(!text_row.contains("accept_unpreviewable"));
}

#[tokio::test]
async fn a_text_preview_is_inert() {
    let s = server();
    let response = s.get("/approvals/cmd_text/attachments/0").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_of(&response, header::CONTENT_TYPE), "text/plain; charset=utf-8");
    assert_eq!(header_of(&response, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert_eq!(header_of(&response, header::CACHE_CONTROL), "no-store");
    assert!(header_of(&response, header::CONTENT_SECURITY_POLICY).contains("sandbox"));
    assert_eq!(text(response).await, "<script>alert(1)</script>");
}

#[tokio::test]
async fn an_image_preview_uses_only_the_detected_type() {
    let s = server();
    let response = s.get("/approvals/cmd_image/attachments/0").await;
    assert_eq!(header_of(&response, header::CONTENT_TYPE), "image/png");
    assert!(header_of(&response, header::CONTENT_DISPOSITION).starts_with("inline"));
}

#[tokio::test]
async fn anything_else_only_downloads_under_a_neutral_name() {
    let s = server();
    let response = s.get("/approvals/cmd_binary/attachments/0").await;
    assert_eq!(header_of(&response, header::CONTENT_TYPE), "application/octet-stream");
    assert_eq!(
        header_of(&response, header::CONTENT_DISPOSITION),
        "attachment; filename=\"attachment-1.bin\""
    );
}

#[tokio::test]
async fn a_preview_needs_an_owner_session() {
    let s = server();
    let response = s
        .router
        .clone()
        .oneshot(
            Request::get("/approvals/cmd_text/attachments/0")
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(s.get("/approvals/cmd_text/attachments/x").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(s.get("/approvals/cmd_text/attachments/3").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_decision_carries_the_reviewed_binding_and_consent() {
    let s = server();
    let response = s
        .post(
            "/approvals/cmd_binary",
            "decision=approve&attachments=binding-binary&accept_unpreviewable=1",
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        s.fake.decided.lock().unwrap().as_slice(),
        [("cmd_binary".to_owned(), true, "binding-binary".to_owned(), true)]
    );
}
