# Apple Bridge for Mac

Your Mac's Apple data (Messages, Calendar, Reminders, Contacts, Notes, Screen Time) as a live, fully synced, read/write copy on any computer, with one REST API and one MCP server.

This repository owns the Rust gateway, native Mac Helper, consent pages, relay and gateway-side iPhone integration. The native iPhone app, widgets, extensions, client-side fixtures, signed builds and TestFlight tooling live in [apple-bridge-iphone](https://github.com/JohanSardar/apple-bridge-iphone).

## Mac development

Use `make build` to build the gateway and native Helper. `make lint`, `make features` and `make rust-ci` check this repository. `make sim`, `make real` and `make replica` run the scenarios through the Rust lab runner. Mac builds need no iPhone checkout or Xcode iOS SDK. On the Air, run build and test steps serially through `~/bin/ab-local`.

## iPhone integration

The gateway still owns pairing, `/v1/iphone/*`, command admission, the ledger, signed record ingestion and relay/APNs delivery. The iPhone repository owns device execution, permissions, keys and receipts. Their wire protocol and installed bundle IDs are unchanged.

Only scenarios that compile phone models or drive the phone harness need a checkout of the app. Clone it separately and set its location explicitly:

```sh
git clone https://github.com/JohanSardar/apple-bridge-iphone.git ../apple-bridge-iphone
APPLE_BRIDGE_IPHONE_ROOT=/path/to/apple-bridge-iphone ~/bin/ab-local make sim SCENARIOS=lab/scenarios/iphone/APP-SHELL-SIM.yaml
```

Gateway pairing and delivery checks still run here without the app checkout. The integrated [feature graph](features.yaml) links app-owned modules to the iPhone repository. App build instructions and physical qualification history moved with the app.

Status: in development. Design: [design/](design/). Research: [research/](research/).

The full SolidJS web app lives on the `web-app` branch. Main serves only consent, passkey enrollment and agent approvals; agent management uses `bridge agent`.
