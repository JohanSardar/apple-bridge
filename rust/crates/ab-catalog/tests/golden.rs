use ab_catalog::{mcp_tools, openapi, operations};
use serde_json::Value;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn all_descriptors_equal_published_contract() -> Result {
    let descriptors = operations()?;
    assert_eq!(descriptors.len(), 158);
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
fn mcp_tool_list_equal_go_json_and_sse() -> Result {
    for raw in [
        include_str!("../../../../contracts/golden/mcp-json.json"),
        include_str!("../../../../contracts/golden/mcp-sse.json"),
    ] {
        let transcript: Value = serde_json::from_str(raw)?;
        let exchange = &transcript["exchanges"][2];
        let response = exchange["response"].as_str().ok_or("response")?;
        let json = if response.starts_with("event:") {
            response
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .ok_or("SSE data")?
        } else {
            response
        };
        let response: Value = serde_json::from_str(json)?;
        assert_eq!(mcp_tools(&operations()?), response["result"]);
    }
    Ok(())
}
