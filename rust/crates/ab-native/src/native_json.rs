use ab_wire::gojson::{self, EscapeMode};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("non-finite native JSON number")]
    NonFinite,
    #[error("native JSON nesting limit")]
    Limit,
    #[error("native JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("native record JSON: {0}")]
    Wire(#[from] gojson::Error),
}

pub fn number(value: f64) -> Result<String, Error> {
    if !value.is_finite() {
        return Err(Error::NonFinite);
    }
    if value == 0.0 {
        return Ok(if value.is_sign_negative() { "-0" } else { "0" }.into());
    }
    let serialized = serde_json::to_string(&value)?;
    let negative = serialized.starts_with('-');
    let unsigned = serialized.trim_start_matches('-');
    let (base, exponent) = unsigned
        .split_once('e')
        .map_or((unsigned, 0), |(b, e)| (b, e.parse::<i32>().unwrap_or(0)));
    let point = base.find('.').unwrap_or(base.len());
    let raw = base.replace('.', "");
    let leading = raw.len() - raw.trim_start_matches('0').len();
    let digits = raw.trim_start_matches('0').trim_end_matches('0');
    let exponent = exponent + i32::try_from(point).map_err(|_| Error::Limit)?
        - i32::try_from(leading).map_err(|_| Error::Limit)?
        - 1;
    let sign = if negative { "-" } else { "" };
    if !(-4..16).contains(&exponent) || value.abs() > 9_007_199_254_740_992.0 {
        let tail = if digits.len() > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        return Ok(format!(
            "{sign}{}{tail}e{}{:02}",
            &digits[..1],
            if exponent < 0 { "-" } else { "+" },
            exponent.unsigned_abs()
        ));
    }
    let point = exponent + 1;
    if point <= 0 {
        return Ok(format!(
            "{sign}0.{}{digits}",
            "0".repeat(usize::try_from(-point).map_err(|_| Error::Limit)?)
        ));
    }
    let point = usize::try_from(point).map_err(|_| Error::Limit)?;
    if point >= digits.len() {
        return Ok(format!(
            "{sign}{digits}{}",
            "0".repeat(point - digits.len())
        ));
    }
    Ok(format!("{sign}{}.{}", &digits[..point], &digits[point..]))
}

pub fn description(value: f64) -> Result<String, Error> {
    let mut text = number(value)?;
    if !text.contains(['.', 'e']) {
        text.push_str(".0");
    }
    Ok(text)
}

fn encode(value: &Value, slashes: bool, depth: usize) -> Result<String, Error> {
    if depth > 128 {
        return Err(Error::Limit);
    }
    match value {
        Value::Null => Ok("null".into()),
        Value::Bool(v) => Ok(v.to_string()),
        Value::Number(v) => {
            if v.is_f64() {
                number(v.as_f64().ok_or(Error::NonFinite)?)
            } else {
                Ok(v.to_string())
            }
        }
        Value::String(v) => {
            let text = serde_json::to_string(v)?;
            Ok(if slashes {
                text.replace('/', "\\/")
            } else {
                text
            })
        }
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(|v| encode(v, slashes, depth + 1))
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        )),
        Value::Object(values) => Ok(format!(
            "{{{}}}",
            values
                .iter()
                .map(|(k, v)| Ok(format!(
                    "{}:{}",
                    encode(&Value::String(k.clone()), slashes, depth + 1)?,
                    encode(v, slashes, depth + 1)?
                )))
                .collect::<Result<Vec<_>, Error>>()?
                .join(",")
        )),
    }
}
pub fn foundation_data(value: &Value) -> Result<Vec<u8>, Error> {
    Ok(encode(value, true, 0)?.into_bytes())
}
pub fn record_data(value: &Value) -> Result<Vec<u8>, Error> {
    Ok(gojson::record_data(
        encode(value, false, 0)?.as_bytes(),
        EscapeMode::Html,
    )?)
}
