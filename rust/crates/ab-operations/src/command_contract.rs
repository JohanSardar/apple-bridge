use crate::{Code, Command, Error};
pub use ab_commands::{ActionContext, NativeFile, Validated};
pub fn command(c: &Command) -> Result<ab_commands::Command, Error> {
    Ok(ab_commands::Command {
        id: c.id.clone(),
        op: c.op.clone(),
        args: c.args.clone(),
        expected_version: c.expected_version,
        wait_ms: i64::try_from(c.wait_ms).map_err(|_| Code::Invalid)?,
    })
}
fn error(e: ab_commands::Error) -> Error {
    match e {
        ab_commands::Error::Invalid => Code::Invalid,
        ab_commands::Error::Unsupported => Code::Unsupported,
        ab_commands::Error::TooLarge => Code::TooLarge,
        ab_commands::Error::Expired => Code::Expired,
    }
    .into()
}
pub fn validate(c: &Command) -> Result<Validated, Error> {
    ab_commands::validate(&command(c)?, jiff::Timestamp::now()).map_err(error)
}
pub fn action(
    c: &Command,
    context: &ActionContext,
) -> Result<Box<serde_json::value::RawValue>, Error> {
    let bytes = ab_commands::native_action(&command(c)?, context).map_err(error)?;
    serde_json::value::RawValue::from_string(String::from_utf8(bytes).map_err(|_| Code::Corrupt)?)
        .map_err(|_| Code::Corrupt.into())
}
pub fn expiry(
    op: &str,
    now: jiff::Timestamp,
    requested: Option<jiff::Timestamp>,
) -> Result<jiff::Timestamp, Error> {
    ab_commands::expiry(op, now, requested).map_err(error)
}
pub fn prepared(
    c: &Command,
    raw: &serde_json::value::RawValue,
) -> Result<Box<serde_json::value::RawValue>, Error> {
    let mut context: ActionContext = serde_json::from_str(raw.get()).map_err(|_| Code::Corrupt)?;
    context.origin.clone_from(&c.origin);
    let fields: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_str(raw.get()).map_err(|_| Code::Corrupt)?;
    if c.op.starts_with("messages.")
        && let Some(messages) = fields.get("messages")
    {
        let messages: serde_json::Value =
            serde_json::from_str(messages.get()).map_err(|_| Code::Corrupt)?;
        if let Some(files) = messages.get("files") {
            context.files = serde_json::from_value(files.clone()).map_err(|_| Code::Corrupt)?;
        }
        if let Some(resend) = messages["resend_of"].as_str() {
            context.resend_key = resend.into();
        }
    }
    let mut command = c.clone();
    if (crate::outcome::capture(&c.op) || c.op == "pdf.annotate")
        && let Some(args) = fields.get("args")
    {
        command.args = args.clone();
    }
    action(&command, &context)
}
pub fn serialization_key(c: &Command) -> Result<String, Error> {
    let target = validate(c)?;
    if c.op.starts_with("files.") {
        return Ok("files".into());
    }
    if c.op.starts_with("notes.") {
        if !target.id.is_empty() {
            return Ok(format!("notes:{}", target.id));
        }
        let encoded = action(c, &ActionContext::default())?;
        let action: serde_json::Value =
            serde_json::from_str(encoded.get()).map_err(|_| Code::Corrupt)?;
        return Ok(format!(
            "notes::{}",
            action["note_args"]["folder_id"]
                .as_str()
                .unwrap_or_default()
        ));
    }
    if c.op.starts_with("calendar.") && target.id.is_empty() {
        return Ok(format!("calendar:{}", target.list_id));
    }
    Ok(if !target.id.is_empty() {
        target.id
    } else if !target.list_id.is_empty() {
        target.list_id
    } else {
        "lists".into()
    })
}
