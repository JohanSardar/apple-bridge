# Go command oracle

Run at the repository root on a build host:

```sh
python3 tools/command-vectors/inputs.py > /tmp/ab-command-inputs.json
go run ./tools/command-vectors < /tmp/ab-command-inputs.json > rust/crates/ab-commands/vectors.json
go run ./tools/command-vectors policy > rust/crates/ab-commands/policy-vectors.json
cargo test --manifest-path rust/Cargo.toml -p ab-commands -p ab-wire
```

The Go oracle calls the product contracts validators and marshals typed NativeAction and IPCFrame values. It does not import Rust or infer expected verdicts from schemas. The synthetic inputs cover source operations, required fields, unknown fields, nulls, wrong types, versions, command IDs and wait limits. Successful Helper actions and complete Prepare IPC frames are compared as bytes, including Go declaration order, pointer presence, zero-value omission, raw contact argument escapes and HTML escaping. iPhone commands enter encrypted delivery and do not produce Helper Prepare actions.

The policy corpus calls Go's Notes HTML and clock functions and reproduces the expiry calculation from LocalGateway.Submit after the stored-command lookup. Focus deadlines and alarm fire times use far-future inputs; Rust validation takes an explicit clock.

The unchanged Helper remains the oracle for Apple-dependent route proofs, lab tags, recipient ownership, native recurrence count-to-end conversion and variant capability. These corpora exercise gateway contracts and encoding; they do not certify Helper native effects or command lifecycle integration.
