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

lint:
	python3 packaging/dev/test_source_registrations.py
	gofmt -l $$(find . -name '*.go' -not -path './.worktrees/*' -not -path '*/node_modules/*' -not -path './.build/*') | grep -v '^$$' && exit 1 || true
	go vet ./...
	go run ./tools/lint .

features:
	go run ./tools/featuregraph -html design/features.html

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
	GOOS=darwin GOARCH=arm64 go build -o bin/bridge-darwin-arm64 ./cmd/bridge
	GOOS=linux GOARCH=amd64 go build -o bin/bridge-linux-amd64 ./cmd/bridge
	GOOS=linux GOARCH=arm64 go build -o bin/bridge-linux-arm64 ./cmd/bridge

.PHONY: decoder-checks
decoder-checks:
	packaging/dev/heavy.sh packaging/dev/check-decoders.sh

.PHONY: rust-check rust-lint
rust-check:
	rust/check.sh check --workspace

rust-lint:
	rust/check.sh fmt --all -- --check
	rust/check.sh clippy --workspace --all-targets -- -D warnings
	rust/check.sh deny check

.PHONY: rust-test rust-ci
rust-test:
	rust/check.sh test --workspace --locked

rust-ci: lint rust-test rust-lint
	go test ./contracts/golden ./internal/frontdoor ./internal/iphone ./internal/web -run Golden -count=1
