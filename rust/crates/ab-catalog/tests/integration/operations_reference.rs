use ab_catalog::{operations, operations_reference_markdown};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

const DOC: &str = include_str!("../../../../../docs/reference/operations.md");

#[test]
fn reference_doc_matches_registry() -> Result {
    let descriptors = operations()?;
    let rendered = operations_reference_markdown(&descriptors);
    assert_eq!(
        rendered, DOC,
        "docs/reference/operations.md is stale; regenerate with `cargo run -p ab-catalog --bin operations-reference`"
    );
    for descriptor in descriptors {
        let heading = format!("### {}", descriptor.name);
        let count = DOC.lines().filter(|line| *line == heading).count();
        assert_eq!(count, 1, "{} must appear exactly once", descriptor.name);
    }
    Ok(())
}
