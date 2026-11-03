use crate::Error;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_ACTION: &str = "is.workflow.actions.output";
const TOKEN_STRING: &str = "WFTextTokenString";
const TOKEN_ATTACHMENT: &str = "WFTextTokenAttachment";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workflow {
    version: i64,
    name: String,
    actions: Option<Vec<Action>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    key: String,
    identifier: String,
    parameters: Option<Map<String, Value>>,
}

#[derive(Debug)]
pub struct Artifact {
    pub source: String,
    pub unsigned: String,
    pub signed: Option<String>,
    pub mode: String,
    pub sha256: String,
}

fn safe_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[must_use]
pub fn stable_id(name: &str) -> String {
    let h = Sha256::digest(name.as_bytes());
    let hex = |bytes: &[u8]| {
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02X}");
            s
        })
    };
    format!(
        "{}-{}-{}-{}-{}",
        hex(&h[..4]),
        hex(&h[4..6]),
        hex(&h[6..8]),
        hex(&h[8..10]),
        hex(&h[10..16])
    )
}

fn invalid<T>(text: &str) -> Result<T, Error> {
    Err(Error::message(text))
}

fn reference(
    map: &Map<String, Value>,
    name: &str,
    outputs: &Map<String, Value>,
) -> Result<Option<Value>, Error> {
    let mut fields = 1;
    if map.get("$property").is_some_and(|v| !v.is_null()) {
        fields += 1;
    }
    if map.get("$text") == Some(&Value::Bool(true)) {
        fields += 1;
    }
    if map.get("$coerce").is_some_and(|v| !v.is_null()) {
        fields += 1;
    }
    if map.len() != fields {
        return Ok(None);
    }
    if let Some(key) = map.get("$output").and_then(Value::as_str) {
        let Some(id) = outputs.get(key).and_then(Value::as_str) else {
            return Err(Error::Message(format!("unknown or forward output {key:?}")));
        };
        let mut value = json!({"Type": "ActionOutput", "OutputUUID": id, "OutputName": key});
        let mut aggrandizements = Vec::new();
        if let Some(property) = map
            .get("$property")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
        {
            aggrandizements.push(json!({"Type": "WFPropertyVariableAggrandizement", "PropertyName": property, "PropertyUserInfo": property}));
        }
        if let Some(coercion) = map
            .get("$coerce")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
        {
            aggrandizements.push(
                json!({"Type": "WFCoercionVariableAggrandizement", "CoercionItemClass": coercion}),
            );
        }
        if !aggrandizements.is_empty() {
            value["Aggrandizements"] = Value::Array(aggrandizements);
        }
        if map.get("$text") == Some(&Value::Bool(true)) {
            return Ok(Some(
                json!({"Value": {"string": "\u{fffc}", "attachmentsByRange": {"{0, 1}": value}}, "WFSerializationType": TOKEN_STRING}),
            ));
        }
        return Ok(Some(
            json!({"Value": value, "WFSerializationType": TOKEN_ATTACHMENT}),
        ));
    }
    if let Some(group) = map
        .get("$group")
        .and_then(Value::as_str)
        .filter(|g| !g.is_empty())
    {
        return Ok(Some(Value::String(stable_id(&format!(
            "{name}/group/{group}"
        )))));
    }
    if map.get("$input") == Some(&Value::Bool(true)) {
        return Ok(Some(
            json!({"Value": {"Type": "ExtensionInput"}, "WFSerializationType": TOKEN_ATTACHMENT}),
        ));
    }
    Ok(None)
}

fn resolve(value: &Value, name: &str, outputs: &Map<String, Value>) -> Result<Value, Error> {
    match value {
        Value::Object(map) => {
            if let Some(resolved) = reference(map, name, outputs)? {
                return Ok(resolved);
            }
            let mut out = Map::new();
            for (key, item) in map {
                if key.starts_with('$') {
                    return Err(Error::Message(format!("invalid reference {key:?}")));
                }
                out.insert(key.clone(), resolve(item, name, outputs)?);
            }
            if out.get("WFSerializationType") == Some(&Value::String(TOKEN_STRING.into()))
                && let Some(Value::Object(text)) = out.get_mut("Value")
                && let Some(Value::Object(attachments)) = text.get_mut("attachmentsByRange")
            {
                for item in attachments.values_mut() {
                    let inner = item
                        .as_object()
                        .filter(|t| {
                            t.get("WFSerializationType")
                                == Some(&Value::String(TOKEN_ATTACHMENT.into()))
                        })
                        .and_then(|t| t.get("Value").cloned());
                    if let Some(inner) = inner {
                        *item = inner;
                    }
                }
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| resolve(item, name, outputs))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

fn escape(buffer: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => buffer.push_str("&#34;"),
            '\'' => buffer.push_str("&#39;"),
            '&' => buffer.push_str("&amp;"),
            '<' => buffer.push_str("&lt;"),
            '>' => buffer.push_str("&gt;"),
            '\t' => buffer.push_str("&#x9;"),
            '\n' => buffer.push_str("&#xA;"),
            '\r' => buffer.push_str("&#xD;"),
            '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => {
                buffer.push(c);
            }
            _ => buffer.push('\u{fffd}'),
        }
    }
}

fn element(buffer: &mut String, tag: &str, text: &str) {
    buffer.push('<');
    buffer.push_str(tag);
    buffer.push('>');
    escape(buffer, text);
    buffer.push_str("</");
    buffer.push_str(tag);
    buffer.push_str(">\n");
}

#[must_use]
pub fn format_float(value: f64) -> String {
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if value.is_sign_negative() && value != 0.0 {
        "-"
    } else {
        ""
    };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if !(-4..6).contains(&exponent) {
        let mut text = format!("{sign}{}", &digits[..1]);
        if digits.len() > 1 {
            text.push('.');
            text.push_str(&digits[1..]);
        }
        let magnitude = exponent.abs();
        let _ = write!(
            text,
            "e{}{magnitude:02}",
            if exponent < 0 { '-' } else { '+' }
        );
        return text;
    }
    let point = exponent + 1;
    let mut text = String::from(sign);
    if point <= 0 {
        text.push_str("0.");
        text.push_str(&"0".repeat(usize::try_from(-point).unwrap_or(0)));
        text.push_str(&digits);
    } else if point >= count {
        text.push_str(&digits);
        text.push_str(&"0".repeat(usize::try_from(point - count).unwrap_or(0)));
    } else {
        let split = usize::try_from(point).unwrap_or(0);
        text.push_str(&digits[..split]);
        text.push('.');
        text.push_str(&digits[split..]);
    }
    text
}

fn plist_value(buffer: &mut String, value: &Value) -> Result<(), Error> {
    match value {
        Value::String(text) => element(buffer, "string", text),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                element(buffer, "integer", &integer.to_string());
            } else if let Some(real) = number.as_f64() {
                element(buffer, "real", &format_float(real));
            } else {
                return invalid("invalid number");
            }
        }
        Value::Bool(true) => buffer.push_str("<true/>\n"),
        Value::Bool(false) => buffer.push_str("<false/>\n"),
        Value::Array(items) => {
            buffer.push_str("<array>\n");
            for item in items {
                plist_value(buffer, item)?;
            }
            buffer.push_str("</array>\n");
        }
        Value::Object(map) => {
            buffer.push_str("<dict>\n");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                element(buffer, "key", key);
                plist_value(buffer, &map[key])?;
            }
            buffer.push_str("</dict>\n");
        }
        Value::Null => return invalid("unsupported plist value <nil>"),
    }
    Ok(())
}

pub fn compile(path: &Path) -> Result<Vec<u8>, Error> {
    let plist = compile_plist(path)?;
    let mut buffer = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n",
    );
    plist_value(&mut buffer, &plist)?;
    buffer.push_str("</plist>\n");
    Ok(buffer.into_bytes())
}

pub fn compile_plist(path: &Path) -> Result<Value, Error> {
    let raw = fs::read(path)?;
    let mut stream = serde_json::Deserializer::from_slice(&raw);
    let source = Workflow::deserialize(&mut stream)?;
    if stream.end().is_err() {
        return invalid("expected one JSON document");
    }
    let actions_source = match source.actions {
        Some(actions) if source.version == 1 && !source.name.trim().is_empty() => actions,
        _ => return invalid("version 1, name and actions required"),
    };
    let mut outputs = Map::new();
    let mut actions = Vec::new();
    let mut has_input = false;
    for action in &actions_source {
        let parameters = action.parameters.clone().unwrap_or_default();
        if !safe_name(&action.key)
            || action.identifier.is_empty()
            || outputs
                .get(&action.key)
                .is_some_and(|v| v.as_str() != Some(""))
        {
            return invalid("unique action key and identifier required");
        }
        if parameters.contains_key("UUID") {
            return invalid("UUID is generated from action key");
        }
        let Value::Object(mut converted) =
            resolve(&Value::Object(parameters), &source.name, &outputs)?
        else {
            return invalid("invalid parameters");
        };
        let id = stable_id(&format!("{}/action/{}", source.name, action.key));
        converted.insert("UUID".into(), Value::String(id.clone()));
        let serialized = serde_json::to_string(&converted)?;
        has_input = has_input || serialized.contains("\"Type\":\"ExtensionInput\"");
        outputs.insert(action.key.clone(), Value::String(id));
        actions.push(json!({"WFWorkflowActionIdentifier": action.identifier, "WFWorkflowActionParameters": converted}));
    }
    let mut plist = json!({
        "WFWorkflowName": source.name,
        "WFWorkflowActions": actions,
        "WFWorkflowClientVersion": "2600.0.0",
        "WFWorkflowMinimumClientVersion": 900,
        "WFWorkflowMinimumClientVersionString": "900",
        "WFWorkflowIcon": {"WFWorkflowIconStartColor": 4_282_601_983_i64, "WFWorkflowIconGlyphNumber": 59511},
        "WFWorkflowTypes": [],
        "WFWorkflowInputContentItemClasses": ["WFStringContentItem", "WFDictionaryContentItem"],
        "WFWorkflowImportQuestions": [],
        "WFWorkflowHasShortcutInputVariables": has_input,
    });
    if actions_source.iter().any(|a| a.identifier == OUTPUT_ACTION) {
        plist["WFWorkflowHasOutputAction"] = Value::Bool(true);
    }
    Ok(plist)
}

fn run_with_deadline(command: &mut Command, deadline: Instant) -> Result<bool, Error> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::DeadlineExceeded);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn sign(input: &Path, output: &Path, mode: &str, deadline: Instant) -> Result<String, Error> {
    let modes: Vec<&str> = if mode == "auto" {
        vec!["anyone", "people-who-know-me"]
    } else {
        vec![mode]
    };
    for candidate in &modes {
        let _ = fs::remove_file(output);
        let signed = run_with_deadline(
            Command::new("/usr/bin/shortcuts")
                .args(["sign", "--mode", candidate, "--input"])
                .arg(input)
                .arg("--output")
                .arg(output),
            deadline,
        );
        match signed {
            Ok(true) => {
                let data = fs::read(output).unwrap_or_default();
                if data.len() < 4 || &data[..4] != b"AEA1" {
                    let _ = fs::remove_file(output);
                    return invalid("signer did not produce an AEA archive");
                }
                return Ok((*candidate).to_owned());
            }
            Err(Error::DeadlineExceeded) => {
                let _ = fs::remove_file(output);
                return Err(Error::DeadlineExceeded);
            }
            Ok(false) | Err(_) => {}
        }
    }
    let _ = fs::remove_file(output);
    Err(Error::Message(format!(
        "shortcuts sign failed in requested mode(s): {}",
        modes.join(", ")
    )))
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub source: String,
    pub out: PathBuf,
    pub unsigned: bool,
    pub mode: String,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            source: String::new(),
            out: PathBuf::from(".build/shortcuts"),
            unsigned: false,
            mode: "auto".into(),
        }
    }
}

fn go_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[must_use]
pub fn manifest(artifacts: &[Artifact]) -> String {
    let mut out = String::from("[\n");
    for (index, a) in artifacts.iter().enumerate() {
        out.push_str("  {\n");
        let _ = writeln!(out, "    \"source\": {},", go_string(&a.source));
        let _ = writeln!(out, "    \"unsigned\": {},", go_string(&a.unsigned));
        if let Some(signed) = &a.signed {
            let _ = writeln!(out, "    \"signed\": {},", go_string(signed));
        }
        let _ = writeln!(out, "    \"mode\": {},", go_string(&a.mode));
        let _ = writeln!(out, "    \"unsigned_sha256\": {}", go_string(&a.sha256));
        out.push_str(if index + 1 == artifacts.len() {
            "  }\n"
        } else {
            "  },\n"
        });
    }
    out.push_str("]\n");
    out
}

fn sources(options: &BuildOptions) -> Vec<String> {
    if !options.source.is_empty() {
        return vec![options.source.clone()];
    }
    let mut paths: Vec<String> = fs::read_dir("packaging/shortcuts")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

pub fn build(options: &BuildOptions) -> Result<String, Error> {
    if !["auto", "anyone", "people-who-know-me"].contains(&options.mode.as_str()) {
        return invalid("invalid build arguments or signing mode");
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    let paths = sources(options);
    if paths.is_empty() {
        return invalid("no shortcut sources");
    }
    fs::create_dir_all(&options.out)?;
    match fs::remove_file(options.out.join("manifest.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let mut artifacts = Vec::new();
    let mut names = std::collections::HashSet::new();
    for path in &paths {
        let base = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = base.strip_suffix(".json").unwrap_or(&base).to_owned();
        if !safe_name(&stem) {
            return invalid("invalid source filename");
        }
        let data = compile(Path::new(path)).map_err(|e| Error::Message(format!("{path}: {e}")))?;
        let display: Value = serde_json::from_slice(&fs::read(path)?)?;
        let name = display
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if name.trim() != name || name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
            return invalid("invalid display name");
        }
        if !names.insert(name.to_owned()) {
            return invalid("duplicate display name");
        }
        let unsigned = options.out.join(format!("{stem}.unsigned.shortcut"));
        fs::write(&unsigned, &data)?;
        let mut artifact = Artifact {
            source: path.clone(),
            unsigned: unsigned.to_string_lossy().into_owned(),
            signed: None,
            mode: "unsigned".into(),
            sha256: Sha256::digest(&data)
                .iter()
                .fold(String::new(), |mut s, b| {
                    let _ = write!(s, "{b:02x}");
                    s
                }),
        };
        if !options.unsigned {
            let signed = options.out.join(format!("{name}.shortcut"));
            artifact.mode = sign(&unsigned, &signed, &options.mode, deadline)?;
            artifact.signed = Some(signed.to_string_lossy().into_owned());
        }
        artifacts.push(artifact);
    }
    let text = manifest(&artifacts);
    fs::write(options.out.join("manifest.json"), &text)?;
    Ok(text)
}

pub fn open(path: &Path) -> Result<(), Error> {
    if path.extension().is_none_or(|x| x != "shortcut") {
        return invalid("open requires one signed .shortcut file");
    }
    let path = std::path::absolute(path)?;
    let raw = fs::read(&path)?;
    if raw.len() < 4 || &raw[..4] != b"AEA1" {
        return invalid("expected a signed AEA shortcut");
    }
    let status = Command::new("/usr/bin/open").arg(&path).status()?;
    if status.success() {
        Ok(())
    } else {
        invalid("open failed")
    }
}
