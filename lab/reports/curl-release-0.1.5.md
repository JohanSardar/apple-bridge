# Curl release 0.1.5 qualification

Date: 2026-11-09 UTC. Verdict: signed release, Eilmer default onboarding route,
and installation from the public curl URL on a clean Mac pass. The independent
parent review accepted the installer bytes and this qualification with the limits
below. No product behavior changed beyond the merged source and release version.

## Artifact and source

- Release: <https://github.com/JohanSardar/apple-bridge/releases/tag/v0.1.5>.
- Build source: `fd4ea1c635aafb6a3d00ac8d9cabb2c7013c9a20`, including merged
  main `82a6da62755f34bdc07fe587bf73fa660e4a4c75`. Later installer/report commits
  do not change the compiled source.
- Archive: `applebridge-0.1.5.tgz`, 14,548,739 bytes.
- Archive SHA-256:
  `fb6432d1022826e0a05f4317a9fc3601790fde6a9905b95892d0b32cb20b89d6`.
- Installer SHA-256:
  `56f4ee806d39a08333b6caada1559a954fc1723201df1c57660664615541c0e2`.
- Existing bundle identifiers and signing team remain unchanged.

The build used an authorized signing Mac, an isolated checkout, a private Cargo
target directory and four Cargo jobs. Release binaries were compiled only on the authorized signing Mac. Apple accepted the notarization submission. Stapling and validation
passed. The extracted app passed strict nested code requirements, hardened
runtime, version and byte-for-byte app-file parity checks. Gatekeeper reported
`accepted` and `source=Notarized Developer ID`.

The public GitHub asset was downloaded independently with `curl -fsSL` and its
SHA-256 matched the archive above before either installer advanced. The flat curl
app has no Sparkle updater or update-feed signature. Updates rerun the installer,
which pins the archive checksum; Developer ID signatures seal the app contents.

## Public installer rollout

The parent independently reviewed the canonical and retained owner Pages PR
bytes. Only the production static installer file was atomically replaced. Its
previous bytes, owner and mode were retained for rollback. No relay runtime,
service or configuration changed or restarted.

Owner Pages deployment reported `built` for merge
`eb2bab4b36672bb25fa8ba3ca29a91d9fb6297c3`. Both public URLs returned exactly
the installer digest above and matched canonical `docs/install`:

```sh
curl -fsSL https://eilmer.si/applebridge/install -o canonical-install
curl -fsSL https://johansardar.github.io/apple-bridge/install -o retained-install
shasum -a 256 canonical-install retained-install
cmp canonical-install retained-install
```

## Actual clean Mac

A fresh clone of the existing authorized macOS base ran macOS 26.6.2. Before
installation, SIP was enabled and Gatekeeper reported assessments enabled and
Developer ID enabled. The install directory, application state, command symlink
and Applications copy were absent. This was a separate macOS guest, not a
redirected HOME. No security settings or native grants were changed.

The guest ran the public command without replacing its URL or archive:

```sh
curl -fsSL https://eilmer.si/applebridge/install | sh
~/.local/bin/bridge --version
codesign --verify --deep --strict "$HOME/.applebridge/Apple Bridge.app"
xcrun stapler validate "$HOME/.applebridge/Apple Bridge.app"
spctl -a -vv --type execute "$HOME/.applebridge/Apple Bridge.app"
~/.local/bin/bridge up
~/.local/bin/bridge status
~/.local/bin/bridge doctor
curl -fsS http://127.0.0.1:7780/v1/health
~/.local/bin/bridge remote status
~/.local/bin/bridge permissions status
```

Installation reported success and `bridge 0.1.5`. Installed-app signature,
staple and Gatekeeper checks passed. First startup reported 0.3 seconds;
health returned `{"state":"alive"}`. Status reported gateway role. Doctor
reported `ok=true`, SQLite integrity complete, `sqlite=ok`, private modes true
and Keychain credentials. Remote access was initially off. Protected sources
reported missing permissions; no permission guide or request was run.

With no saved relay override or relay environment override, `bridge remote on`
produced an onboarding URL on `app.applebridge.eilmer.si`. The command was bounded
to 20 seconds and timed out waiting for owner sign-in. Remote status was pending,
backend relay, without a last error. This proves the default onboarding route,
not successful enrollment. The source default control endpoint is
`https://applebridge.eilmer.si`.

After `bridge remote off`, `bridge down` and `bridge up`, remote remained off,
startup reported 0.2 seconds and health again returned alive. SIP and Gatekeeper
still reported the original enabled settings.

## Checks and limits

On the authorized build Mac, the final release tree passed `make lint`,
`make features` and `make rust-lab-independence`. Focused packaging tests passed
34 cases; release Python tests passed 12 cases. `sh -n docs/install`, locked
workspace metadata at version 0.1.5, staged-site installer parity and public
installer parity passed. Feature graph: 79 features, 78 done, one in progress;
434/475 scenario-level entries passing. Existing graph limits were not resolved.

The clean guest's authenticated enrollment and customer TLS were not qualified.
Native permission-dependent reads were not qualified because no new grants were
made. Doctor observed FileVault off in the guest; it remained unchanged. Physical
iPhone continuity and future certificate renewal were not requalified. Prior
relay migration evidence is separate and is not claimed as fresh guest evidence.
