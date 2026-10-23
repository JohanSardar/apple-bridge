ifeq ($(APPLE_BRIDGE_KEEP_SDKROOT),)
MATCHING_SDKROOT := $(shell packaging/dev/sdkroot.sh)
ifneq ($(MATCHING_SDKROOT),)
export SDKROOT := $(MATCHING_SDKROOT)
endif
endif

export PATH := /opt/homebrew/bin:$(HOME)/go/bin:$(PATH)

.PHONY: build lint features sim real replica agent dist native

build: web
	packaging/dev/heavy.sh go build -p 2 ./...
	packaging/dev/heavy.sh packaging/dev/build-native.sh

native:
	packaging/dev/heavy.sh packaging/dev/build-native.sh

lint:
	cd web && pnpm lint
	gofmt -l $$(find . -name '*.go' -not -path './.worktrees/*' -not -path '*/node_modules/*' -not -path './.build/*') | grep -v '^$$' && exit 1 || true
	go vet ./...
	go run ./tools/lint .

features:
	cd web && pnpm exec node --test dev/scenarios.test.mjs && pnpm exec node dev/scenarios.mjs
	go run ./tools/featuregraph -html design/features.html

sim:
	go run ./cmd/bridge-lab run lab/scenarios --level sim

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

.PHONY: web
web:
	packaging/dev/heavy.sh sh -c 'cd web && pnpm install --frozen-lockfile && pnpm build:release'

.PHONY: demo
demo: web
	packaging/dev/heavy.sh go run ./cmd/bridge-lab demo
