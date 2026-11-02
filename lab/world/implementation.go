package world

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

type Implementations struct {
	Prebuilt      bool     `json:"-"`
	Gateway       string   `json:"gateway"`
	Replica       string   `json:"replica"`
	Replicas      []string `json:"replicas,omitempty"`
	RustBinary    string   `json:"-"`
	LocalReplicas bool     `json:"-"`
}

type implementationsKey struct{}

func WithImplementations(ctx context.Context, selection Implementations) context.Context {
	return context.WithValue(ctx, implementationsKey{}, selection)
}

func ImplementationSelection(ctx context.Context) Implementations {
	selection, _ := ctx.Value(implementationsKey{}).(Implementations)
	if selection.Gateway == "" {
		selection.Gateway = "go"
	}
	if selection.Replica == "" {
		selection.Replica = "go"
	}
	return selection
}

func ParseImplementations(value string) (Implementations, error) {
	selection := Implementations{Gateway: "go", Replica: "go"}
	seen := map[string]bool{}
	for _, entry := range strings.Split(value, ",") {
		role, impl, ok := strings.Cut(entry, "=")
		if !ok || seen[role] || impl != "go" && impl != "rust" {
			return selection, fmt.Errorf("invalid implementation %q", entry)
		}
		seen[role] = true
		switch role {
		case "gateway":
			selection.Gateway = impl
		case "replica":
			selection.Replica = impl
		default:
			return selection, fmt.Errorf("unknown implementation role %q", role)
		}
	}
	return selection, nil
}

func (s Implementations) ReplicaAt(index int) string {
	if index < len(s.Replicas) {
		return s.Replicas[index]
	}
	return s.Replica
}

func (s Implementations) Binary(impl string, goBinary string) (string, error) {
	if impl == "go" {
		return goBinary, nil
	}
	if impl != "rust" {
		return "", fmt.Errorf("unsupported implementation %q", impl)
	}
	path := s.RustBinary
	if path == "" {
		return "", fmt.Errorf("impl=rust blocked: supply --rust-binary with the bridge executable from cargo build --message-format=json")
	}
	path, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	info, err := os.Stat(path)
	if err != nil {
		return "", fmt.Errorf("impl=rust blocked: %w", err)
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0111 == 0 {
		return "", fmt.Errorf("Rust bridge is not executable: %s", path)
	}
	return path, nil
}

func (w *World) endpointBinary(e Endpoint) string {
	if e.Binary != "" {
		return e.Binary
	}
	if e.Remote {
		return w.remoteBinary
	}
	return w.goBridge
}
