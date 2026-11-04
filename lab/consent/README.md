# Consent browser proof

Build a fresh gateway binary and `lab-simapple` through `~/bin/ab-local`, then run:

```sh
AB_PROOF_BRIDGE=/absolute/path/to/bridge AB_PROOF_LAB=/absolute/path/to/lab-simapple AB_PROOF_PLAYWRIGHT=/absolute/path/to/installed/playwright ~/bin/ab-local node lab/consent/proof.cjs
```

This uses Chromium's CDP virtual authenticator, a fresh temporary home, the file secret store, and a SimApple peer. All command effects are synthetic. It exercises the production gateway and creates no native Apple data. HTTP, callback and gateway peer ports are allocated dynamically. The script shuts down its own processes and saves consent, pending approvals and the server-rendered final command status to `~/ab-live/proof/web-removal/`. Private temporary state stays outside git for inspection.
