# Capture configuration

ab-session captures every source from the user's home with no configuration; the Helper starts no capture. The
file `session/rust-capture.json` under the Helper home is only an override: per-source fields merge over the
defaults, and `"exclusive": true` limits a run to the named sources.

These scripts edit that file. They remain until the live switch script stops calling them. `unflip` can no longer
return a source to Swift, because Swift has no capture code: removing an entry only returns it to its defaults.

    packaging/capture/flip-capture.sh safari,usage --home DIR [--helper-app APP] [--dry-run] [--restart-helper]
    packaging/capture/unflip-capture.sh safari --home DIR [--restart-helper]

`DIR` is the gateway home. The scripts refuse the live home (`~/Library/Application Support/AppleBridgeLive`)
unless `AB_FLIP_LIVE=yes`. `packaging/capture/check.sh` (part of `make lint`) runs them against a temporary home and
checks that the source list equals `production::SUPPORTED` in ab-session.
