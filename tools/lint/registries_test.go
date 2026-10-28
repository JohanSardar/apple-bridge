package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestSourceRegistries(t *testing.T) {
	for _, f := range lintSourceRegistries(filepath.Join("..", "..")) {
		t.Errorf("%s:%d: %s", f.path, f.line, f.msg)
	}
}

func TestDescriptorFileBoundaries(t *testing.T) {
	for _, tc := range []struct {
		path, body string
		fail       bool
	}{
		{"internal/contracts/operations.go", `var d = OperationDescriptor{Name: "new.read", Source: New}`, true},
		{"internal/contracts/new_descriptors.go", `var d = OperationDescriptor{Name: "new.read", Source: New}; func init(){registerDescriptors(NewDescriptors)}`, false},
		{"internal/contracts/new_descriptors.go", `var d = OperationDescriptor{Name: "other.read", Source: Other}; func init(){registerDescriptors(NewDescriptors)}`, true},
		{"internal/contracts/new_descriptors.go", `var d = OperationDescriptor{Name: "new.read", Source: New}`, true},
		{"internal/contracts/commands.go", `func Descriptor() OperationDescriptor {return OperationDescriptor{}}`, false},
		{"internal/contracts/new.go", `func init(){Operations = append(Operations, d)}`, true},
	} {
		if got := len(lintDescriptorFile(tc.path, []byte("package contracts;"+tc.body))) > 0; got != tc.fail {
			t.Errorf("%s rejected=%v want %v", tc.path, got, tc.fail)
		}
	}
}

func TestBootstrapRegistrations(t *testing.T) {
	for _, tc := range []struct {
		path, body string
		fail       bool
	}{
		{"lab/simapple/commands.go", `package simapple; func init(){sourceCommands = append(sourceCommands, (*fakeApple).routeNewCommand)}`, true},
		{"lab/simapple/commands.go", `package simapple; func command(){f.routeNewCommand()}`, true},
		{"lab/simapple/commands.go", `package simapple; func command(){for _, route := range sourceCommands {route()}}`, false},
		{"native/Sources/AppleSession/HelperCommand.swift", `await SessionSources.register(into: peer, home: home, labEnabled: true); try? await peer.register(NewSource())`, true},
		{"native/Sources/AppleSession/HelperCommand.swift", `await SessionSources.register(into: peer, home: home, labEnabled: true)`, false},
	} {
		if got := len(lintBootstrapRegistrations(tc.path, []byte(tc.body))) > 0; got != tc.fail {
			t.Errorf("%s %s: rejected=%v want %v", tc.path, tc.body, got, tc.fail)
		}
	}
}

func TestSharedBootstrapInjection(t *testing.T) {
	for _, tc := range []struct{ path, injection string }{
		{"lab/simapple/commands.go", "\nfunc init(){sourceCommands = append(sourceCommands, (*fakeApple).routeNewCommand)}\n"},
		{"native/Sources/AppleSession/HelperCommand.swift", "\nfunc injected(){try? await peer.register(NewSource())}\n"},
	} {
		t.Run(filepath.Base(tc.path), func(t *testing.T) {
			root := t.TempDir()
			for _, rel := range []string{"lab/simapple/simapple.go", "lab/simapple/commands.go", "native/Package.swift", "native/Sources/AppleSession/HelperCommand.swift", "native/Sources/AppleSession/SessionSources.swift"} {
				body, err := os.ReadFile(filepath.Join("..", "..", rel))
				if err != nil {
					t.Fatal(err)
				}
				if rel == tc.path {
					body = append(body, []byte(tc.injection)...)
				}
				destination := filepath.Join(root, rel)
				if err := os.MkdirAll(filepath.Dir(destination), 0700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(destination, body, 0600); err != nil {
					t.Fatal(err)
				}
			}
			for _, f := range lintSourceRegistries(root) {
				if f.path == tc.path {
					return
				}
			}
			t.Fatal("injected source registration passed lint")
		})
	}
}
