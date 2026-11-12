use ab_catalog::{mcp_tools, openapi, operations};
use serde_json::Value;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn all_descriptors_equal_published_contract() -> Result {
    let descriptors = operations()?;
    assert_eq!(descriptors.len(), 161);
    let expected: Value =
        serde_json::from_str(include_str!("../../../../contracts/golden/catalog.json"))?;
    assert_eq!(serde_json::to_value(descriptors)?, expected);
    Ok(())
}

#[test]
fn openapi_equal_go() -> Result {
    let expected: Value =
        serde_json::from_str(include_str!("../../../../contracts/golden/openapi.json"))?;
    assert_eq!(openapi(&operations()?), expected);
    Ok(())
}

#[test]
fn legacy_mcp_tool_list_is_unchanged() -> Result {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../contracts/golden/mcp-legacy-tools.json"
    ))?;
    assert_eq!(mcp_tools(&operations()?), expected);
    Ok(())
}
