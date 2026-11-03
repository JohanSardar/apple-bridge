# Narrow WebAuthn compatibility patches

Sources: crates.io webauthn-rs-core 0.6.2 (MPL-2.0, Michael/William Brown and
upstream contributors, https://github.com/kanidm/webauthn-rs) and crypto-glue
0.2.2 (MPL-2.0, William Brown and upstream contributors,
https://github.com/kanidm/crypto-glue). Original package READMEs and manifests
retain attribution. MPL-2.0 applies to these sources and their modifications:
https://www.mozilla.org/en-US/MPL/2.0/ .

Patches remove crypto-glue's RSA dependency, exports and RSA X.509 verification;
WebAuthn rejects RSA and verifies Ed25519 with ed25519-dalek::verify_strict.
Only Ed25519 (not Ed448) is accepted for OKP keys. The application's COSE policy
restricts imports and registration to ES256/P-256 and EdDSA/Ed25519.

Upstream comments and dependency unit tests were removed for repository source
policy; missing-doc warnings are disabled accordingly. Source grammar, other
ceremony checks and credential types remain upstream. Product interoperability
and frozen ceremony proofs live outside these dependencies. Registry caches,
lockfiles, generated documentation and build outputs are excluded.
