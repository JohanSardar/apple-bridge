# Web app foundations

Build on the Air with `make web`. Node and pnpm are contributor tools; the production app contains only static assets consumed by `web/embed.go`.

`AB_WEB_MOCK=1 pnpm dev` opts into the fixture API and WebSocket plugin. Visit `/__mock/session` first to create a trusted fixture session, or `/__mock/session?kind=public` for a public session, then open `/messages`. Without this environment variable Vite does not register any fixture endpoint. No fixture server code is imported by browser source or emitted into `dist`.

Run browser scenarios through `packaging/dev/heavy.sh go run ./cmd/bridge-lab run lab/scenarios/web --level web --json`. Each YAML declares its browser, theme, layout and server matrix. Mock scenarios exercise frontend behavior; gateway scenarios must exercise the actual Go front door and never fall back to fixture authentication. Playwright runs one worker and fails fast. Reports and measured timing samples are under `lab/evidence/web`; screenshots and failure traces remain private.

`pnpm test:math` uses Node's built-in test runner and type stripping. It adds no dependency and is limited to analytic spring and position math, using independent numerical derivatives and prefix sums.

The virtual list accepts stable row keys, an optional height estimate, bottom anchoring, a render function and a handle exposing scroll-to-index and the current anchor. Height measurements are cached by stable key. Prepending preserves the visible key and offset; bottom anchoring follows appends only when already at the bottom. Arrow, Home, End and Page keys expose an active row to assistive technology.

`LiveClient.subscribe` returns a reactive accessor and disposer. Snapshots replace views; deltas support replace, upsert and removal. Reconnect sends saved cursors. Commands are admitted only synchronously inside `withInput(event, callback)` while a trusted input event is being dispatched. They fail immediately offline. Reconnect observes existing command records rather than redispatching effects. `pending()` exposes optimistic payloads and reconciled statuses to app components.

The logical protocol is `web/1`; its HTTP WebSocket subprotocol token is `web.1`.

Trusted sessions may keep at most 100 cached views and a 200 MiB thumbnail LRU. The service worker grants caching only after verifying the current session, and validates navigation sessions before using a precached shell. Public sessions use memory, no service worker registration, no IndexedDB, no Cache Storage, and no persisted appearance or scroll state. Session cookies end at browser close; public idle expiry is also enforced by the gateway. Browser unload is not treated as sign-out because the same event occurs on reload and deep-link navigation.

Fonts are self-hosted Inter under OFL. The font stack resolves Apple system fonts first; Inter is not preloaded or precached. The line sprite uses Lucide-compatible geometry under the included ISC license and ships no Apple artwork.
