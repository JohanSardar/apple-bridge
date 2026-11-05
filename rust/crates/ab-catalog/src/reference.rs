use crate::OperationDescriptor;
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Write as _};

pub fn operations_reference_markdown(descriptors: &[OperationDescriptor]) -> String {
    let mut groups: BTreeMap<String, Vec<&OperationDescriptor>> = BTreeMap::new();
    for descriptor in descriptors {
        groups
            .entry(source_name(descriptor))
            .or_default()
            .push(descriptor);
    }
    let mut out = String::new();
    out.push_str("# Operations reference\n\n");
    out.push_str(
        "Generated from the operation registry in `rust/crates/ab-catalog` with `cargo run -p ab-catalog --bin operations-reference`. Do not edit by hand.\n\n",
    );
    out.push_str(
        "This lists the catalog only: whether an operation is usable depends on its source, granted permissions, and the feature status in `features.yaml`.\n\n",
    );
    let _ = writeln!(out, "{} operations.\n", descriptors.len());
    for (source, operations) in &groups {
        let _ = writeln!(out, "## {source}\n");
        for descriptor in operations {
            render_operation(&mut out, descriptor);
        }
    }
    out
}

fn source_name(descriptor: &OperationDescriptor) -> String {
    descriptor.source.map_or_else(
        || "gateway".to_string(),
        |source| {
            serde_json::to_value(source)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_string())
        },
    )
}

fn render_operation(out: &mut String, descriptor: &OperationDescriptor) {
    let _ = writeln!(out, "### {}\n", descriptor.name);
    let _ = writeln!(out, "{}\n", descriptor.description);
    let kind = match descriptor.kind {
        crate::OperationKind::Query => "query",
        crate::OperationKind::Read => "read",
        crate::OperationKind::Command => "command",
    };
    let scope = if descriptor.scope.is_empty() {
        "none".to_string()
    } else {
        format!("`{}`", descriptor.scope)
    };
    let _ = writeln!(out, "- Kind: `{kind}`\n- Scope: {scope}\n");
    render_parameters(out, &descriptor.input);
    out.push('\n');
}

fn render_parameters(out: &mut String, input: &Value) {
    let Some(properties) = input.get("properties").and_then(Value::as_object) else {
        out.push_str("No parameters.\n");
        return;
    };
    if properties.is_empty() {
        out.push_str("No parameters.\n");
        return;
    }
    let required: Vec<&str> = input
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    out.push_str("| Parameter | Type | Required | Description |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for (name, schema) in properties {
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} | {} |",
            type_text(schema),
            if required.contains(&name.as_str()) {
                "yes"
            } else {
                "no"
            },
            cell(description_text(schema)),
        );
    }
}

fn type_text(schema: &Value) -> String {
    let base = match schema.get("type") {
        Some(Value::String(kind)) if kind == "array" => schema
            .pointer("/items/type")
            .and_then(Value::as_str)
            .map_or_else(|| "array".to_string(), |inner| format!("array<{inner}>")),
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ => {
            for key in ["oneOf", "anyOf"] {
                if let Some(variants) = schema.get(key).and_then(Value::as_array) {
                    return variants
                        .iter()
                        .map(type_text)
                        .collect::<Vec<_>>()
                        .join(" | ");
                }
            }
            "value".to_string()
        }
    };
    if schema.get("nullable").and_then(Value::as_bool) == Some(true) {
        format!("{base} | null")
    } else {
        base
    }
}

fn description_text(schema: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        parts.push(description.to_string());
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let options = values
            .iter()
            .map(|value| match value {
                Value::String(text) => format!("`{text}`"),
                other => format!("`{other}`"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("One of: {options}"));
    }
    parts.join(" ")
}

fn cell(text: String) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}
