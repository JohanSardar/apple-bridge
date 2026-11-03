use super::raw;
use crate::peer::IpcFrame;

const GO_PREPARED_EVIDENCE: &str = r#"{"command_id":"cmd_shadow_mcp","expires_at":"0001-01-01T00:00:00Z","ticket":"ticket_01M6HB6A63TF3YMKJ0X5F5NP9R","route":"sim-reminders","fingerprint":"e7dfee035dddf6fedf90fa8cbe1a495caef3386a002fc00c9380b4566e0de214","candidate_ids":[""],"t":"Prepared"}"#;

#[test]
fn prepared_evidence_uses_go_struct_order() {
    let prepared: IpcFrame = serde_json::from_str(GO_PREPARED_EVIDENCE).unwrap();
    assert_eq!(raw(&prepared).unwrap().get(), GO_PREPARED_EVIDENCE);
}

#[test]
fn clipboard_arguments_are_still_redacted() {
    let record = serde_json::json!({"command": {"op": "mac.clipboard_write", "args": {"text": "secret"}}});
    let redacted = raw(&record).unwrap();
    assert!(!redacted.get().contains("secret"));
}
