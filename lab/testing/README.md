# Scenario runner

`packaging/dev/rust-lab.sh` builds `bridge` (`lab-queries`), `lab-families`, `lab-simapple`, `ab-session` and the relay fixtures from `rust/`, then runs scenarios. `make sim`, `make real`, `make replica`, `make families`, `make sim-parallel`, `make gateway-matrix` and `make replica-matrix` wrap it. No scenario YAML is changed by these tools.

`lab/testing/sim_parallel.py` is the parallel runner. It needs `--rust-binary`, `--families-binary` and `--simapple-binary`, takes a `--level sim|replica`, and runs each selected scenario in its own `lab-families run` process with its own World, random paths and ephemeral ports. The parent applies a hard deadline (900 seconds by default), sends TERM, allows ten seconds of cleanup, then kills the process group.

`lab/test-selection.json` is a frozen map of every scenario: level, host, feature, Swift modules and the `run-at-end` classification. It is no longer regenerated; its Go `code_paths`, `roots` and Go package selectors were removed with the Go lab. `--select` takes scenario IDs, feature IDs, crate names and Swift module names, and repeated selectors form a union.

`make sim` runs the default set, which excludes `run-at-end` entries. Run those serially with `make sim-run-at-end SIM_OUTPUT=/absolute/new/end-directory` on each host. Explicit selectors may run either set. `SIM_JOBS`, `SIM_TIMEOUT`, `SIM_SELECT`, `SIM_OUTPUT` and `RUST_BRIDGE` are the overrides.

`lab/no-runner.json` lists scenarios that no runner executes (`AGENT-SUITE`, `QUERY-AGENT`, `RMT-ONECOMMAND-FLOW`, `RMT-TS-EMBEDDED`), each with the reason. `sim_parallel.py` and `rust-lab.sh` report them as `no-runner` with that reason and do not count them as failures. The agent level has no runner: `make agent` names them and exits 1.

The gateway matrix runs on Linux only (`make gateway-matrix` on `ssh build`). Set `APPLE_BRIDGE_LAB_ROOT` to a private disk-backed directory when `/tmp` has a quota. Worlds use that root; the default is `/tmp/ablab`. Logs and summaries use `SIM_OUTPUT` independently.

Set `APPLE_BRIDGE_IPHONE_ROOT` to the sibling iPhone checkout for its integration fixtures. Native fixture runs use `python3 lab/testing/sim_parallel.py --parallel 1` with the three binaries. The host filter defaults to the current OS; `--host all` attempts everything. Each Air scenario is wrapped in `~/bin/ab-local`, releasing the shared lock between scenarios, and the Air never runs native SIM scenarios concurrently.

`TEST-INVENTORY.md` records the historical measurement gaps and classifications. Real sets are proposals only; none of these commands executes real scenarios unless `make real` is given explicit scenarios.
