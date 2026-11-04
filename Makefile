ifeq ($(APPLE_BRIDGE_KEEP_SDKROOT),)
MATCHING_SDKROOT := $(shell packaging/dev/sdkroot.sh)
ifneq ($(MATCHING_SDKROOT),)
export SDKROOT := $(MATCHING_SDKROOT)
endif
endif

export PATH := /opt/homebrew/bin:$(HOME)/go/bin:$(PATH)

.PHONY: build lint features sim real replica agent dist native

build:
	packaging/dev/heavy.sh go build -p 2 ./...
	packaging/dev/heavy.sh packaging/dev/build-native.sh

native:
	packaging/dev/heavy.sh packaging/dev/build-native.sh

lint: lint-rust-sources
	python3 packaging/dev/test_source_registrations.py
	gofmt -l $$(find . -name '*.go' -not -path './.worktrees/*' -not -path '*/node_modules/*' -not -path './.build/*') | grep -v '^$$' && exit 1 || true
	go vet ./...
	go run ./tools/lint .

.PHONY: lint-rust-sources
lint-rust-sources:
	rust/check.sh run -q -p ab-lint -- $(CURDIR)

features:
	rust/check.sh run -q -p ab-featuregraph -- -root $(CURDIR) -html $(CURDIR)/design/features.html

SCENARIOS ?= lab/scenarios

sim: native
	go run ./cmd/bridge-lab run $(SCENARIOS) --level sim

real:
	go run ./cmd/bridge-lab run lab/scenarios --level real

replica:
	go run ./cmd/bridge-lab run lab/scenarios --level replica

agent:
	go run ./cmd/bridge-lab run lab/scenarios --level agent

dist:
	mkdir -p bin
	packaging/dev/build-bridge.sh bin/bridge-darwin-arm64 aarch64-apple-darwin
	packaging/dev/build-bridge.sh bin/bridge-linux-amd64 x86_64-unknown-linux-musl
	packaging/dev/build-bridge.sh bin/bridge-linux-arm64 aarch64-unknown-linux-musl
	packaging/dev/build-bridge.sh bin/ab-relay-linux-amd64 x86_64-unknown-linux-musl ab-relay ab-relay
	packaging/dev/build-bridge.sh bin/ab-relay-linux-arm64 aarch64-unknown-linux-musl ab-relay ab-relay

.PHONY: decoder-checks
decoder-checks:
	packaging/dev/heavy.sh packaging/dev/check-decoders.sh

.PHONY: rust-check rust-lint
rust-check:
	rust/check.sh check --workspace

rust-lint: lint-rust-sources
	rust/check.sh fmt --all -- --check
	rust/check.sh clippy --workspace --all-targets -- -D warnings
	rust/check.sh clippy -p ab-session --all-targets --features rust-effects,rust-capture -- -D warnings
	rust/check.sh deny check

.PHONY: rust-test rust-search-perf rust-ci
rust-test:
	rust/check.sh test --workspace --locked
	rust/check.sh test -p ab-session --features rust-effects,rust-capture --locked

rust-search-perf:
	rust/check.sh test --release --locked -p ab-state --test search_update_perf -- --ignored --nocapture

rust-ci: lint rust-test rust-search-perf rust-lint rust-lab-independence
	python3 rust/check-native-graph.py
	go test ./contracts/golden ./internal/frontdoor ./internal/iphone ./internal/web ./lab/scenarios -run Golden -count=1

.PHONY: replica-matrix replica-matrix-go
REPLICA_MATRIX_FLAGS ?= --local-replicas --jobs 4
REPLICA_IMPL ?= gateway=go,replica=rust
RUST_BRIDGE ?=
replica-matrix:
	REPLICA_IMPL="$(REPLICA_IMPL)" REPLICA_MATRIX_FLAGS="$(REPLICA_MATRIX_FLAGS)" RUST_BRIDGE="$(RUST_BRIDGE)" SCENARIOS="$(SCENARIOS)" packaging/dev/replica-matrix.sh

replica-matrix-go:
	go run ./cmd/bridge-lab replica-matrix $(SCENARIOS) --impl gateway=go,replica=go $(REPLICA_MATRIX_FLAGS)

.PHONY: sim-parallel
SIM_JOBS ?= $(if $(filter Darwin,$(shell uname -s)),1,4)
SIM_TIMEOUT ?= 900
SIM_OUTPUT ?= /tmp/ab-sim-parallel-$(shell date +%s)
SIM_SELECT ?=
sim-parallel:
	python3 lab/testing/sim_parallel.py --parallel $(SIM_JOBS) --timeout $(SIM_TIMEOUT) --output $(SIM_OUTPUT) $(if $(SIM_SELECT),--select $(SIM_SELECT),)

.PHONY: sim-run-at-end
sim-run-at-end:
	python3 lab/testing/sim_parallel.py --parallel 1 --run-at-end --timeout $(SIM_TIMEOUT) --output $(SIM_OUTPUT) $(if $(SIM_SELECT),--select $(SIM_SELECT),)

.PHONY: gateway-matrix
gateway-matrix:
	RUST_BRIDGE="$(RUST_BRIDGE)" SIM_JOBS="$(SIM_JOBS)" SIM_TIMEOUT="$(SIM_TIMEOUT)" SIM_OUTPUT="$(SIM_OUTPUT)" SIM_SELECT="$(SIM_SELECT)" packaging/dev/gateway-matrix.sh
.PHONY: rust-lab-independence
rust-lab-independence:
	rust/lab/check-independence.sh
