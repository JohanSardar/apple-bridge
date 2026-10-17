export PATH := /opt/homebrew/bin:$(HOME)/go/bin:$(PATH)

.PHONY: build lint features sim real replica agent dist native

build:
	packaging/dev/heavy.sh go build -p 2 ./...
	packaging/dev/heavy.sh packaging/dev/build-native.sh

native:
	packaging/dev/heavy.sh packaging/dev/build-native.sh

lint:
	gofmt -l . | grep -v '^$$' && exit 1 || true
	go vet ./...
	go run ./tools/lint .

features:
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
