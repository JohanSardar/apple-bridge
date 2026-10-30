# Web scenario iteration

Run one browser, theme and viewport while diagnosing a scenario:

```sh
AB_WEB_PROJECTS=firefox-dark-desktop go run ./cmd/bridge-lab run lab/scenarios/web/WEB-MSG-MEDIA.yaml --level web
```

`AB_WEB_PROJECTS` accepts comma-separated Playwright project names and intersects them with the scenario's matrix. A filter matching no projects fails before starting a fixture. Omit it for the full matrix.

`WEB-MSG-ATTACH-SEND` and `WEB-MSG-FAIL-RESEND` automatically use a lab-tagged runner so their upload checkpoints and exact-once effect counts remain available. The production build keeps those control endpoints absent.

Failure screenshots are always retained. Set `AB_WEB_TRACE=1` to also record Playwright traces; tracing every action and DOM snapshot adds overhead even to successful tests.

Set `AB_WEB_PLATES=1` to save the optional media presentation plates. Pixel-comparison screenshots and failure screenshots run regardless of this flag.
