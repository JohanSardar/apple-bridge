use ab_frontdoor::web::{AttachmentFile, Reviewed};
use ab_operations::{
    Code,
    approval_review::{ApprovalReview, OwnerReview, Preview, display_arguments},
    local::LocalGateway,
};
use ab_trust::{Error, Principal};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug)]
pub struct Approvals(pub Arc<LocalGateway>);
fn error(value: &ab_operations::Error) -> Error {
    match value.code {
        Code::Unauthorized | Code::Forbidden => Error::Unauthorized,
        Code::NotFound | Code::Invalid => Error::Invalid,
        _ => Error::Unavailable,
    }
}
fn pending_row(
    record: &ab_operations::local::CommandRecord,
    review: Result<ApprovalReview, ab_operations::Error>,
) -> Result<Value, Error> {
    let mut row = json!({"command_id":record.command.id, "operation":record.command.op,
        "principal":record.principal,
        "arguments":serde_json::to_string_pretty(&json!({"argument_bytes":record.command.args.get().len(),"argument_hash":ab_trust::fingerprint(record.command.args.get().as_bytes())}))?,
        "owner_arguments":display_arguments(record).map_err(|value| error(&value))?});
    match review {
        Ok(review) => {
            row["attachments"] = serde_json::to_value(&review.attachments)?;
            row["binding"] = review.binding.into();
            row["requires_consent"] = review.requires_consent.into();
        }
        Err(_) => row["attachment_error"] = true.into(),
    }
    Ok(row)
}
impl ab_frontdoor::web::Approvals for Approvals {
    fn pending(&self) -> Result<Vec<Value>, Error> {
        let runtime = tokio::runtime::Handle::current();
        self.0
            .approvals()
            .map_err(|value| error(&value))?
            .iter()
            .map(|record| pending_row(record, runtime.block_on(self.0.approval_review(record))))
            .collect()
    }
    fn command(&self, id: &str) -> Result<Value, Error> {
        let row = self.0.state.command(id).map_err(|value| error(&value))?;
        let row = self
            .0
            .approvals()
            .map_err(|value| error(&value))?
            .into_iter()
            .find(|record| record.command.id == id)
            .unwrap_or(row);
        Ok(serde_json::to_value(row)?)
    }
    fn attachment(&self, id: &str, index: usize) -> Result<AttachmentFile, Error> {
        let (attachment, bytes) = tokio::runtime::Handle::current()
            .block_on(self.0.approval_attachment(id, index))
            .map_err(|value| error(&value))?;
        Ok(AttachmentFile {
            media_type: attachment.media_type,
            preview: match attachment.preview {
                Preview::Text => "text",
                Preview::Image => "image",
                Preview::DownloadOnly | Preview::TooLarge => "download_only",
            }
            .into(),
            bytes,
        })
    }
    fn decide(
        &self,
        owner: &Principal,
        id: &str,
        approve: bool,
        reviewed: &Reviewed<'_>,
    ) -> Result<Value, Error> {
        let owner = ab_operations::Principal {
            id: owner.id.clone(),
            kind: owner.kind.clone(),
            grants: owner
                .grants
                .iter()
                .map(|grant| grant.as_str().to_owned())
                .collect(),
            level: owner.level.clone(),
            resource: String::new(),
            remote: owner.remote,
            expires_at: None,
        };
        let result = tokio::runtime::Handle::current()
            .block_on(self.0.decide(
                &owner,
                id,
                approve,
                &OwnerReview {
                    binding: reviewed.binding,
                    accepted_unpreviewable: reviewed.accepted_unpreviewable,
                },
            ))
            .map_err(|value| error(&value))?;
        Ok(serde_json::from_str(result.get())?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use ab_operations::approval_review::ReviewedAttachment;

    fn record() -> ab_operations::local::CommandRecord {
        serde_json::from_value(json!({"command":{"command_id":"cmd_row","op":"messages.send","args":{"chat_id":"chat_synthetic","files":[{"name":"note.txt","data":"bm90ZQ=="}]}},"principal":"agent:x","source":"messages","request_hash":"hash","state":"awaiting_approval","admission_era":"era","created_at":"2026-01-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z","commit_seq":0,"cursor":{"epoch":"","sequence":0,"subscription_revision":0},"history":[]})).unwrap()
    }

    #[test]
    fn a_pending_row_carries_what_the_approval_page_reads() {
        let row = pending_row(
            &record(),
            Ok(ApprovalReview {
                attachments: vec![ReviewedAttachment {
                    index: 0,
                    name: "note.txt".into(),
                    digest: "d".repeat(64),
                    size: 4,
                    media_type: "text/plain".into(),
                    preview: Preview::DownloadOnly,
                    excerpt: None,
                }],
                binding: "bound".into(),
                requires_consent: true,
            }),
        )
        .unwrap();
        assert_eq!(row["binding"], "bound");
        assert_eq!(row["requires_consent"], true);
        let attachment = &row["attachments"][0];
        assert_eq!(attachment["preview"], "download_only");
        for key in ["index", "name", "digest", "size", "media_type"] {
            assert!(!attachment[key].is_null(), "{key}");
        }
        assert!(!row["owner_arguments"].as_str().unwrap().contains("bm90ZQ=="));
    }

    #[test]
    fn an_unreadable_attachment_is_marked_on_its_row() {
        let row = pending_row(&record(), Err(Code::NotFound.into())).unwrap();
        assert_eq!(row["attachment_error"], true);
        assert!(row["binding"].is_null());
    }
}
