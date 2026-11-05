use ab_catalog::{operations, operations_reference_markdown};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

const DOC: &str = include_str!("../../../../docs/reference/operations.md");

#[test]
fn reference_doc_matches_registry() -> Result {
    let rendered = operations_reference_markdown(&operations()?);
    assert_eq!(
        rendered, DOC,
        "docs/reference/operations.md is stale; regenerate with `cargo run -p ab-catalog --bin operations-reference`"
    );
    Ok(())
}

#[test]
fn every_registry_operation_appears_exactly_once() -> Result {
    for descriptor in operations()? {
        let heading = format!("### {}", descriptor.name);
        let count = DOC.lines().filter(|line| *line == heading).count();
        assert_eq!(count, 1, "{} must appear exactly once", descriptor.name);
    }
    Ok(())
}
