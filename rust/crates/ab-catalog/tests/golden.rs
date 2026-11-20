use ab_catalog::{openapi, operations};
use serde_json::Value;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn all_descriptors_equal_published_contract() -> Result {
    let descriptors = operations()?;
    assert_eq!(descriptors.len(), 138);
    let expected: Value =
        serde_json::from_str(include_str!("../../../../contracts/golden/catalog.json"))?;
    assert_eq!(serde_json::to_value(descriptors)?, expected);
    Ok(())
}

#[test]
fn openapi_matches_published_contract() -> Result {
    let descriptors = operations()?;
    let actual = openapi(&descriptors);
    let expected: Value =
        serde_json::from_str(include_str!("../../../../contracts/golden/openapi.json"))?;
    assert_eq!(actual, expected);
    for descriptor in descriptors {
        let path = format!("/v1/op/{}", descriptor.name);
        let schema = &actual["paths"][path]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"];
        let reference = schema["$ref"]
            .as_str()
            .ok_or("missing response reference")?;
        let pointer = reference.strip_prefix('#').ok_or("non-local reference")?;
        assert_eq!(actual.pointer(pointer), Some(&descriptor.output));
    }
    Ok(())
}

#[test]
fn openapi_uses_caller_supplied_descriptors_alongside_cached_catalog() -> Result {
    let descriptors = operations()?;
    let mut custom = descriptors[..1].to_vec();
    custom[0].description = "Custom description".into();

    let published = openapi(&descriptors);
    let custom_document = openapi(&custom);
    let path = format!("/v1/op/{}", custom[0].name);
    assert_eq!(
        custom_document["paths"][path]["post"]["description"],
        "Custom description"
    );
    assert_eq!(
        custom_document["paths"]
            .as_object()
            .map(serde_json::Map::len),
        Some(2)
    );
    assert_eq!(openapi(&descriptors), published);
    Ok(())
}
