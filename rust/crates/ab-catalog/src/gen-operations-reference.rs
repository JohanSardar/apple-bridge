use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let descriptors = ab_catalog::operations()?;
    let markdown = ab_catalog::operations_reference_markdown(&descriptors);
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/reference/operations.md");
    std::fs::write(&path, markdown)?;
    println!("wrote {}", path.display());
    Ok(())
}
