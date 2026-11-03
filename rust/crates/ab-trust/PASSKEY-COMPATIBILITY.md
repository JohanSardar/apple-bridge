Go's frozen registration transcript (`contracts/golden/passkey.json`) advertises
RS256 (-257), RS384, RS512 and RSA-PSS. The stored credential envelope's
`publicKey` is a CBOR COSE key, and Go accepts RSA keys in that field. Supporting
Go-created homes therefore includes RSA credentials, even though the synthetic
golden stored credential uses ES256. No live credential or home was inspected.

`webauthn-rs-core = 0.6.2` depends on `crypto-glue` with an unconditional `rsa`
dependency. Neither crate exposes an RSA feature toggle. Disabling RSA at the
application algorithm list would not remove the dependency and would break
previously registered RSA credentials.

The RUSTSEC-2023-0071 exception in `rust/deny.toml` is restricted to that advisory.
The affected operation is RSA private-key decryption/signing; this integration
handles only authenticator public keys and signature verification. It neither
loads RSA private keys nor invokes RSA signing/decryption. Remove the exception
when upstream supplies an unaffected implementation or after an explicitly
approved credential migration. The exception does not waive other advisories.
