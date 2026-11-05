use crate::{
    Code, Error,
    domain::CommandState as State,
    local::{CommandRecord, LocalGateway},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;

const MAX_TEXT_PREVIEW: u64 = 1 << 20;
const MAX_INSPECT: u64 = 16 << 20;
const EXCERPT_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preview {
    Text,
    Image,
    DownloadOnly,
    TooLarge,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewedAttachment {
    pub index: usize,
    pub name: String,
    pub digest: String,
    pub size: u64,
    pub media_type: String,
    pub preview: Preview,
    pub excerpt: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApprovalReview {
    pub attachments: Vec<ReviewedAttachment>,
    pub binding: String,
    pub requires_consent: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct OwnerReview<'a> {
    pub binding: &'a str,
    pub accepted_unpreviewable: bool,
}
pub(crate) enum Decider<'a> {
    Owner(&'a OwnerReview<'a>),
    Phone(&'a str),
}
#[derive(Serialize)]
struct Bound<'a> {
    name: &'a str,
    digest: &'a str,
    size: u64,
    media_type: &'a str,
}
#[must_use]
pub fn phone_decision_hash(request_hash: &str, binding: &str) -> String {
    ab_trust::fingerprint(format!("{request_hash}\n{binding}").as_bytes())
}
#[must_use]
pub fn has_attachments(record: &CommandRecord) -> bool {
    !files(record).is_empty()
}
fn files(record: &CommandRecord) -> Vec<Value> {
    if !record.command.op.starts_with("messages.") {
        return Vec::new();
    }
    serde_json::from_str::<Value>(record.command.args.get())
        .ok()
        .and_then(|args| args["files"].as_array().cloned())
        .unwrap_or_default()
}
pub fn display_arguments(record: &CommandRecord) -> Result<String, Error> {
    let mut args: Value =
        serde_json::from_str(record.command.args.get()).map_err(|_| Code::Corrupt)?;
    if record.command.op.starts_with("messages.")
        && let Some(files) = args["files"].as_array_mut()
    {
        for (index, file) in files.iter_mut().enumerate() {
            if file.get("data").is_some() {
                file["data"] = Value::String(format!("<attachment {}>", index + 1));
            }
        }
    }
    serde_json::to_string_pretty(&args).map_err(|_| Code::Corrupt.into())
}
#[must_use]
pub fn phone_summary(review: &ApprovalReview) -> Vec<String> {
    let mut lines = Vec::new();
    for attachment in &review.attachments {
        lines.push(format!(
            "Attachment {}: {}",
            attachment.index + 1,
            attachment.name
        ));
        lines.push(format!(
            "Detected {}, {} bytes",
            attachment.media_type, attachment.size
        ));
        lines.push(format!("SHA-256 {}", attachment.digest));
        if let Some(excerpt) = &attachment.excerpt {
            lines.push(format!("Starts with: {excerpt}"));
        }
        if !previewable(attachment.preview) {
            lines.push(
                "Can't be previewed here. Approving sends these exact bytes; inspect it under Approvals in your browser first."
                    .to_owned(),
            );
        }
    }
    lines
}
const fn previewable(preview: Preview) -> bool {
    matches!(preview, Preview::Text | Preview::Image)
}
fn image_type(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
fn text_of(head: &[u8], whole: bool) -> Option<&str> {
    let valid = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(error) if !whole && error.error_len().is_none() => {
            std::str::from_utf8(&head[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    (!head.is_empty()
        && valid
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r' | '\u{c}')))
    .then_some(valid)
}
fn excerpt_of(text: &str) -> String {
    let words = text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect::<String>();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(EXCERPT_CHARS).collect()
}
fn classify(
    index: usize,
    name: &str,
    digest: String,
    size: u64,
    head: &[u8],
) -> ReviewedAttachment {
    let whole = head.len() as u64 >= size;
    let text = text_of(head, whole);
    let image = image_type(head);
    let media_type = if let Some(image) = image {
        image
    } else if head.starts_with(b"%PDF-") {
        "application/pdf"
    } else if text.is_some() {
        "text/plain"
    } else {
        "application/octet-stream"
    };
    let text = text.filter(|_| media_type == "text/plain");
    let preview = if size > MAX_INSPECT {
        Preview::TooLarge
    } else if image.is_some() {
        Preview::Image
    } else if text.is_some() && size <= MAX_TEXT_PREVIEW {
        Preview::Text
    } else {
        Preview::DownloadOnly
    };
    let name = ab_trust::oauth::normalized_name(name);
    ReviewedAttachment {
        index,
        name: if name.is_empty() {
            "(unnamed)".to_owned()
        } else {
            name
        },
        digest,
        size,
        media_type: media_type.to_owned(),
        preview,
        excerpt: text.map(excerpt_of),
    }
}
fn binding_of(attachments: &[ReviewedAttachment]) -> Result<String, Error> {
    if attachments.is_empty() {
        return Ok(String::new());
    }
    let bound: Vec<Bound<'_>> = attachments
        .iter()
        .map(|a| Bound {
            name: &a.name,
            digest: &a.digest,
            size: a.size,
            media_type: &a.media_type,
        })
        .collect();
    Ok(ab_trust::fingerprint(
        &serde_json::to_vec(&bound).map_err(|_| Code::Corrupt)?,
    ))
}
impl LocalGateway {
    async fn attachment_head(
        &self,
        record: &CommandRecord,
        index: usize,
        file: &Value,
    ) -> Result<ReviewedAttachment, Error> {
        let name = file["name"].as_str().ok_or(Code::Invalid)?;
        if let Some(id) = file["upload_id"].as_str() {
            let store = self.query_store.as_ref().ok_or(Code::Unavailable)?;
            let reference = record.upload_references.get(id).ok_or(Code::NotFound)?;
            let size = u64::try_from(reference.size).map_err(|_| Code::Corrupt)?;
            let head = store
                .read_blob(&reference.digest, 0, MAX_TEXT_PREVIEW.min(size))
                .await?;
            return Ok(classify(index, name, reference.digest.clone(), size, &head));
        }
        let bytes = inline_bytes(file)?;
        Ok(classify(
            index,
            name,
            ab_trust::fingerprint(&bytes),
            bytes.len() as u64,
            &bytes[..bytes.len().min(MAX_TEXT_PREVIEW as usize)],
        ))
    }
    pub async fn approval_review(&self, record: &CommandRecord) -> Result<ApprovalReview, Error> {
        let mut attachments = Vec::new();
        for (index, file) in files(record).iter().enumerate() {
            attachments.push(self.attachment_head(record, index, file).await?);
        }
        Ok(ApprovalReview {
            binding: binding_of(&attachments)?,
            requires_consent: attachments.iter().any(|a| !previewable(a.preview)),
            attachments,
        })
    }
    pub async fn approval_attachment(
        &self,
        id: &str,
        index: usize,
    ) -> Result<(ReviewedAttachment, Vec<u8>), Error> {
        let record = self
            .approvals()?
            .into_iter()
            .find(|record| record.command.id == id)
            .ok_or(Code::NotFound)?;
        let listed = files(&record);
        let file = listed.get(index).ok_or(Code::NotFound)?;
        let attachment = self.attachment_head(&record, index, file).await?;
        if attachment.preview == Preview::TooLarge {
            return Err(Code::TooLarge.into());
        }
        let bytes = if file["upload_id"].is_string() {
            let store = self.query_store.as_ref().ok_or(Code::Unavailable)?;
            store
                .read_blob(&attachment.digest, 0, attachment.size)
                .await?
        } else {
            inline_bytes(file)?
        };
        if ab_trust::fingerprint(&bytes) != attachment.digest {
            return Err(Code::Corrupt.into());
        }
        Ok((attachment, bytes))
    }
    pub(crate) async fn verify_decider(
        &self,
        row: &CommandRecord,
        approve: bool,
        decider: Decider<'_>,
    ) -> Result<(), Error> {
        let binding_applies = approve && row.state == State::AwaitingApproval;
        let review = if has_attachments(row) {
            match self.approval_review(row).await {
                Ok(review) => Some(review),
                Err(error) if binding_applies => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        match decider {
            Decider::Phone(hash) => {
                let matches_bound = review
                    .as_ref()
                    .is_some_and(|r| phone_decision_hash(&row.request_hash, &r.binding) == hash);
                let needs_bound = binding_applies && has_attachments(row);
                if !(matches_bound || !needs_bound && hash == row.request_hash) {
                    return Err(Code::Unauthorized.into());
                }
            }
            Decider::Owner(owner) => {
                if binding_applies
                    && let Some(review) = &review
                    && (owner.binding != review.binding
                        || review.requires_consent && !owner.accepted_unpreviewable)
                {
                    return Err(Code::Conflict.into());
                }
            }
        }
        Ok(())
    }
}
fn inline_bytes(file: &Value) -> Result<Vec<u8>, Error> {
    STANDARD
        .decode(file["data"].as_str().ok_or(Code::Invalid)?)
        .map_err(|_| Code::Invalid.into())
}
