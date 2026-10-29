# Relay qualification — 2026-10-29

Public gateway health: `https://6d6ce3cf9ec7ae9912ad76e2.ab.lexaai.tech/v1/health`.

Independent Ubuntu curl: PASS, 18 response bytes, SHA-256 `5aa61e19a071090c68cb53680088f8b9e534b3d748377dae390a00acd312c86e`. Standard certificate verification; no insecure TLS override. The live gateway reports relay on, with TLS terminating on the Mac.

Control: `https://ab.lexaai.tech`; trusted Let's Encrypt certificate. DNS scope and IAM passed without changes. Only the ab-lexaai-tech zone and ab-relay VM were changed; Cloudflare and production servers were untouched.

Control DNS-01 issuance and renewal reuse the existing ACME implementation, with keys kept in private VM state. Device keys stay on the Mac. DNS publication waits for public propagation and creates an explicit device A record atomically with its challenge TXT. This prevents the challenge's empty DNS parent from suppressing the wildcard A response. Startup retries the initial network request while the outbound tunnel attaches. The control response deadline and streaming scenario deadline now cover their actual work.

Qualification uses isolated synthetic Mac gateways and an external Ubuntu client. Temporary connector credentials and fixture state are removed by the lab cleanup. No private keys, bearer credentials or personal records are included in evidence.

CT completeness remains limited: crt.sh returned zero indexed certificates and subsequently HTTP 502 for the live device during this run. Public certificate trust and relay reachability do not establish complete aggregator coverage. An interactive Claude OAuth consent flow is outside the isolated bearer-authenticated connector check.

Sim qualification (passed assertions / assertions):

- RELAY-PASSTHROUGH: 12/12; PASS.
- RELAY-ENROLL: 12/12; PASS.
- RELAY-LIMITS: 28/28; PASS.
- RELAY-STREAMS: 8/8; PASS.
- RELAY-ACME: 13/13; PASS.
- RELAY-OWN-DOMAIN: 17/17; PASS.

Verification uses the brief-selected SDKROOT. Final verification binaries omit DWARF (`GOFLAGS=-ldflags=-w`) to avoid slow local dsymutil processing; compilation and test behavior are unchanged. Native sources were not changed.

The first final full-suite run hit the unchanged ApplePeer `TestQueryBlockedWrites/cancel` timing failure under high host load. The bounded full-suite retry passed; no ApplePeer sources were changed.

Real qualification (passed assertions / assertions):

- RELAY-REAL: 46/46; PASS.
- CONN-RELAY-CLAUDE: 49/49; PASS.

Required gates: make lint PASS; make features PASS; go build -p 4 ./... PASS; go test -p 4 ./... PASS on bounded retry. No native build required.
