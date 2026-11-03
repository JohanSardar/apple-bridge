use ab_operations::{
    Command,
    command_contract::{self, ActionContext},
};
use ab_trust::wire::{gojson::GoTime, messages::IpcFrame};
use serde::Deserialize;
#[derive(Deserialize)]
struct Vector {
    command: Box<serde_json::value::RawValue>,
    context: ActionContext,
    action: Option<String>,
    prepare: Option<String>,
}
#[test]
fn lifecycle_prepare_preserves_go_codec_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let vectors: Vec<Vector> =
        serde_json::from_str(include_str!("../../ab-commands/vectors.json"))?;
    let mut count = 0;
    for vector in vectors {
        let Some(expected) = vector.action else {
            continue;
        };
        let mut command: Command = serde_json::from_str(vector.command.get())?;
        command.origin = vector.context.origin;
        let action = command_contract::prepared(
            &command,
            &serde_json::value::RawValue::from_string(expected.clone())?,
        )?;
        assert_eq!(action.get(), expected, "{}", command.op);
        if let Some(expected) = vector.prepare {
            let frame = IpcFrame {
                t: "Prepare".into(),
                command_id: command.id,
                principal: "token:fixture".into(),
                request_hash: "hash_fixture".into(),
                action: Some(action),
                expires_at: GoTime::parse("2030-01-01T00:00:00Z")?,
                owner_fence: 7,
                origin: command.origin,
                ..Default::default()
            };
            let bytes = frame.encode().map_err(|e| format!("{e:?}"))?;
            assert_eq!(&bytes[4..], expected.as_bytes(), "{}", command.op);
        }
        count += 1;
    }
    assert!(count >= 540, "compared only {count} Go actions");
    Ok(())
}
#[test]
fn helper_error_codes_remain_go_strings() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = r#"{"code":"native_unavailable","message":"synthetic failure","retryable":false}"#;
    let error: ab_operations::Error = serde_json::from_str(bytes)?;
    assert_eq!(serde_json::to_string(&error)?, bytes);
    Ok(())
}
