# Agents get read, approve-first or full levels

Remote agents can reach the bridge from anywhere, so each agent token carries a level. `read` cannot write. `approve` turns every command into an owner approval in the server-rendered approvals page before anything is admitted. `full` acts like the owner. New tokens defaulted to `approve` until [ADR 0030](0030-connected-apps-act-by-default.md) made `full` the default. Connector UIs use OAuth 2.1 with PKCE, and the owner's device session consents. Lab and test principals keep the original test gates.
