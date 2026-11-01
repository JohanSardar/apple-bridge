package scenarios

import (
	"context"
	"net/url"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func TestRelayReadinessUsesPublicMetadataAndProtectedHealth(t *testing.T) {
	directory := t.TempDir()
	script := "#!/bin/sh\ncase \"$*\" in *\"--max-time 1 --resolve\"*) exit 28;; esac\ncase \"$*\" in\n*v1/health*) case \"$*\" in *--fail*) exit 22;; esac; printf 401;;\n*oauth-authorization-server*) printf '{\"issuer\":\"https://gateway.invalid:10000\"}';;\n*) printf 200;;\nesac\n"
	if err := os.WriteFile(filepath.Join(directory, "ssh"), []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", directory+string(os.PathListSeparator)+os.Getenv("PATH"))
	u, err := url.Parse("https://gateway.invalid:10000")
	if err != nil {
		t.Fatal(err)
	}
	status := remote.Status{URL: u.String()}
	assertions := []Assertion{}
	add := func(name string, pass bool) { assertions = append(assertions, Assertion{Name: name, Passed: pass}) }
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if !relayIngressReady(ctx, status, u, "203.0.113.1", &Evidence{}, add) {
		t.Fatal("public metadata did not establish ingress")
	}
	remotePublicHTTPChecks(ctx, status, u, "203.0.113.1", add)
	if len(assertions) != 3 {
		t.Fatalf("assertions: %+v", assertions)
	}
	for _, assertion := range assertions {
		if !assertion.Passed {
			t.Fatalf("public health must require authentication: %+v", assertions)
		}
	}
}
