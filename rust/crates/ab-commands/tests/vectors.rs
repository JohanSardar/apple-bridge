use ab_commands::{ActionContext, Command, native_action, validate};
use serde::Deserialize;
#[derive(Deserialize)]
struct Vector {
    command: Command,
    context: ActionContext,
    accepted: bool,
    code: String,
    message: String,
    action: Option<String>,
    prepare: Option<String>,
}
#[test]
fn go_validator_and_prepare_corpus() -> Result<(), Box<dyn std::error::Error>> {
    let rows: Vec<Vector> = serde_json::from_str(include_str!("../vectors.json"))?;
    let now = "2026-11-02T12:00:00Z".parse()?;
    let mut errors = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let got = validate(&row.command, now);
        if got.is_ok() != row.accepted
            || got
                .as_ref()
                .err()
                .is_some_and(|e| e.code() != row.code || e.to_string() != row.message)
        {
            errors.push(format!(
                "{index} {} {} expected {} {} got {got:?}",
                row.command.op, row.command.args, row.accepted, row.code
            ));
        }
        if let Some(want) = &row.prepare {
            let ctx = ab_commands::PrepareContext {
                action: &row.context,
                principal: "token:fixture",
                request_hash: "hash_fixture",
                expected_state: None,
                expires_at: ab_wire::gojson::GoTime::parse("2030-01-01T00:00:00Z")?,
                owner_fence: 7,
            };
            assert_eq!(
                ab_commands::prepare(&row.command, &ctx)?,
                want.as_bytes(),
                "prepare {index}"
            );
        }
        if let Some(want) = &row.action {
            match native_action(&row.command, &row.context) {
                Ok(got) if got == want.as_bytes() => {}
                got => errors.push(format!("{index} action expected {want} got {got:?}")),
            }
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.into_iter().take(30).collect::<Vec<_>>().join("\n")
    );
    Ok(())
}
