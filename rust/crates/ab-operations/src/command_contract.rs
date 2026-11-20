use crate::{Code, Command, Error};
pub use ab_commands::{ActionContext, NativeFile, Validated, Violation};
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
    if c.op == "maps.open" {
        let command = command(c)?;
        if !ab_commands::valid_command_id(&command.id) || !(0..=15000).contains(&command.wait_ms) {
            return Err(Code::Invalid.into());
        }
        let args = parse_arguments(c)?;
        crate::native_commands::maps(&args)?;
        return Ok(Validated {
            id: String::new(),
            list_id: String::new(),
            folder_id: String::new(),
        });
    }
    validate_at(c, jiff::Timestamp::now())
}

pub(crate) fn parse_arguments(c: &Command) -> Result<serde_json::Value, Error> {
    parse_json(c.args.get())
}

pub fn parse_json(raw: &str) -> Result<serde_json::Value, Error> {
    ab_commands::parse_arguments(raw).map_err(|violation| violation_error(&violation))
}

pub(crate) fn validate_at(c: &Command, now: jiff::Timestamp) -> Result<Validated, Error> {
    if c.op == "maps.open" {
        return validate(c);
    }
    ab_commands::validate_request(&command(c)?, now)
        .map_err(|violation| violation_error(&violation))
}

fn violation_error(violation: &Violation) -> Error {
    Error {
        code: match violation.code {
            "unsupported" => Code::Unsupported,
            "too_large" => Code::TooLarge,
            "expired" => Code::Expired,
            _ => Code::Invalid,
        },
        message: violation.to_string(),
        current: serde_json::value::to_raw_value(violation).ok(),
        retryable: violation.retryable,
    }
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
    if c.op == "maps.open" {
        return Ok(raw.to_owned());
    }
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
    Ok(serialization_key_for(c, &validate(c)?))
}

pub(crate) fn serialization_key_for(c: &Command, target: &Validated) -> String {
    if c.op.starts_with("files.") {
        return "files".into();
    }
    if c.op.starts_with("notes.") {
        if !target.id.is_empty() {
            return format!("notes:{}", target.id);
        }
        return format!("notes::{}", target.folder_id);
    }
    if c.op.starts_with("calendar.") && target.id.is_empty() {
        return format!("calendar:{}", target.list_id);
    }
    if !target.id.is_empty() {
        target.id.clone()
    } else if !target.list_id.is_empty() {
        target.list_id.clone()
    } else {
        "lists".into()
    }
}
