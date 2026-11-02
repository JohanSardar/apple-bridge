# Port the independent harness last

Status: Accepted; extends ADR 0013.

Retain the Go harness as the oracle while porting the product. Convert each relevant P-class in-process journey and D-class Go home writer to selected-binary execution before accepting evidence for a Rust module. Tag evidence impl=go or impl=rust.

Phase 8 ports lab last, after Go product deletion. Rust lab crates depend on no ab-* product crates and own lab-wire checked against the same goldens. Require matching verdicts over three runs plus seeded checkpoint, fence, ack and scope faults caught by both harnesses. AppleTruth stays Swift as a named exception. See lab/HARNESS-CLASSIFICATION.md and the independently produced lab/TEST-INVENTORY.md.
