# Rust capture switch

`session/rust-capture.json` under the Helper home names the sources whose capture Rust owns. The
bundled `ab-session` (`Contents/MacOS/ab-session`, built by `packaging/dev/build-session.sh` and copied
in by `packaging/build-app.sh` and `packaging/release/release.py`) reads it when the Helper starts.
For every named source the Swift Helper does not start its capture at all, and the switch applies on the
next Helper start. A source stays registered in Swift for commands and facts.

    packaging/capture/flip-capture.sh safari,usage --home DIR [--helper-app APP] [--dry-run] [--restart-helper]
    packaging/capture/unflip-capture.sh safari --home DIR [--restart-helper]
    packaging/capture/unflip-capture.sh all --home DIR

`DIR` is the gateway home. The scripts refuse the live home (`~/Library/Application Support/AppleBridgeLive`)
unless `AB_FLIP_LIVE=yes`. `unflip-capture.sh all` moves the file aside; no file means every source is Swift.
`packaging/capture/check.sh` (part of `make lint`) runs the scripts against a temporary home and checks that
the source list equals `production::SUPPORTED` in ab-session.
