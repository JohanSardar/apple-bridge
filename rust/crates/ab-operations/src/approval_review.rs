use crate::{Error, local::{CommandRecord, LocalGateway}};
use serde::Serialize;

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
pub fn phone_decision_hash(request_hash: &str, _binding: &str) -> String {
    request_hash.to_owned()
}
pub fn display_arguments(record: &CommandRecord) -> Result<String, Error> {
    Ok(record.command.args.get().to_owned())
}
impl LocalGateway {
    pub async fn approval_review(&self, _record: &CommandRecord) -> Result<ApprovalReview, Error> {
        Ok(ApprovalReview::default())
    }
    pub async fn approval_attachment(
        &self,
        _id: &str,
        _index: usize,
    ) -> Result<(ReviewedAttachment, Vec<u8>), Error> {
        Err(crate::Code::NotFound.into())
    }
}
