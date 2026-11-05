ifeq ($(APPLE_BRIDGE_KEEP_SDKROOT),)
MATCHING_SDKROOT := $(shell packaging/dev/sdkroot.sh)
ifneq ($(MATCHING_SDKROOT),)
export SDKROOT := $(MATCHING_SDKROOT)
endif
endif

export PATH := /opt/homebrew/bin:$(PATH)

.PHONY: build lint features sim real replica agent families dist native

RUST_NATIVE_BUILD := $(if $(filter Darwin,$(shell uname -s)),packaging/dev/heavy.sh packaging/dev/build-native.sh,true)

build:
	rust/check.sh build --workspace --locked
	$(RUST_NATIVE_BUILD)

native:
	packaging/dev/heavy.sh packaging/dev/build-native.sh

lint: lint-rust-sources
	python3 packaging/dev/test_source_registrations.py
	packaging/capture/check.sh

.PHONY: lint-rust-sources
lint-rust-sources:
	rust/check.sh run -q -p ab-lint -- $(CURDIR)

features:
	rust/check.sh run -q -p ab-featuregraph -- -root $(CURDIR) -html $(CURDIR)/design/features.html

SCENARIOS ?= lab/scenarios

sim:
	SIM_JOBS="$(SIM_JOBS)" SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/rust-lab.sh sim $(if $(filter-out lab/scenarios,$(SCENARIOS)),$(SCENARIOS),)

real:
	packaging/dev/rust-lab.sh real $(SCENARIOS)

replica:
	SIM_JOBS="$(SIM_JOBS)" SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/rust-lab.sh replica

agent:
	packaging/dev/rust-lab.sh agent

families:
	packaging/dev/rust-lab.sh families $(FAMILIES_FLAGS)

dist:
	mkdir -p bin
	packaging/dev/build-bridge.sh bin/bridge-darwin-arm64 aarch64-apple-darwin
	packaging/dev/build-bridge.sh bin/bridge-linux-amd64 x86_64-unknown-linux-musl
	packaging/dev/build-bridge.sh bin/bridge-linux-arm64 aarch64-unknown-linux-musl
	packaging/dev/build-bridge.sh bin/ab-relay-linux-amd64 x86_64-unknown-linux-musl ab-relay ab-relay
	packaging/dev/build-bridge.sh bin/ab-relay-linux-arm64 aarch64-unknown-linux-musl ab-relay ab-relay

.PHONY: rust-check rust-lint
rust-check:
	rust/check.sh check --workspace

rust-lint: lint-rust-sources
	rust/check.sh fmt --all -- --check
	rust/check.sh clippy --workspace --all-targets -- -D warnings
	rust/check.sh clippy -p ab-session --all-targets -- -D warnings
	rust/check.sh clippy -p ab-session --features lab --all-targets -- -D warnings
	rust/check.sh deny check

.PHONY: rust-test rust-search-perf rust-ci
rust-test:
	rust/check.sh test --workspace --locked
	rust/check.sh test -p ab-session --locked
	rust/check.sh test -p ab-session --locked --features lab

rust-search-perf:
	rust/check.sh test --release --locked -p ab-state --test search_update_perf -- --ignored --nocapture

rust-ci: lint rust-test rust-search-perf rust-lint rust-lab-independence
	python3 rust/check-native-graph.py
	$(MAKE) rust-golden

.PHONY: rust-golden rust-gate
rust-golden:
	rust/check.sh test --workspace --locked golden

rust-gate: lint-rust-sources rust-test rust-lint rust-lab-independence
	python3 rust/check-native-graph.py
	$(MAKE) rust-golden
	python3 packaging/dev/test_source_registrations.py

.PHONY: replica-matrix gateway-matrix sim-parallel sim-run-at-end
REPLICA_MATRIX_FLAGS ?= --local-replicas --jobs 4
RUST_BRIDGE ?=
SIM_JOBS ?= $(if $(filter Darwin,$(shell uname -s)),1,4)
SIM_TIMEOUT ?= 900
SIM_OUTPUT ?= /tmp/ab-sim-parallel-$(shell date +%s)
SIM_SELECT ?=

replica-matrix:
	RUST_BRIDGE="$(RUST_BRIDGE)" REPLICA_MATRIX_FLAGS="$(REPLICA_MATRIX_FLAGS)" SCENARIOS="$(SCENARIOS)" packaging/dev/replica-matrix.sh

gateway-matrix:
	RUST_BRIDGE="$(RUST_BRIDGE)" SIM_JOBS="$(SIM_JOBS)" SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_OUTPUT="$(SIM_OUTPUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/gateway-matrix.sh

sim-parallel:
	SIM_JOBS="$(SIM_JOBS)" SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_OUTPUT="$(SIM_OUTPUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/rust-lab.sh sim

sim-run-at-end:
	SIM_JOBS=1 SIM_RUN_AT_END=1 SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_OUTPUT="$(SIM_OUTPUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/rust-lab.sh sim

.PHONY: rust-lab-independence
rust-lab-independence:
	rust/lab/check-independence.sh
