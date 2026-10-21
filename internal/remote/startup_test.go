package remote

import (
	"context"
	"crypto/tls"
	"net"
	"net/netip"
	"os"
	"path/filepath"
	"tailscale.com/ipn"
	"testing"
)

func TestEmbeddedStartupWithoutNetwork(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, "remote")
	if err := os.Mkdir(dir, 0755); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"", "bridge-test"} {
		node, err := embeddedNode(home, name, func(string, ...any) {})
		if err != nil {
			t.Fatal(err)
		}
		expected := name
		if expected == "" {
			expected = "bridge"
		}
		if node.Dir != dir || node.Hostname != expected || node.UserLogf == nil {
			t.Fatalf("incorrect embedded startup configuration")
		}
		info, err := os.Stat(dir)
		if err != nil || info.Mode().Perm() != 0700 {
			t.Fatalf("remote state must be private")
		}
	}
	for _, name := range []string{"bad.name", "../outside", "-bridge", "bridge-"} {
		if _, err := embeddedNode(home, name, nil); err == nil {
			t.Fatalf("invalid hostname accepted: %q", name)
		}
	}
}

func TestFunnelSourceWithoutNetwork(t *testing.T) {
	a, b := net.Pipe()
	defer a.Close()
	defer b.Close()
	source := netip.MustParseAddrPort("[2001:db8::1234]:54321")
	connection := tls.Server(&ipn.FunnelConn{Conn: a, Src: source}, &tls.Config{})
	ctx := clientConnection(context.Background(), connection)
	if ctx.Value(clientAddressKey{}) != source.String() {
		t.Fatal("Funnel original source was lost")
	}
	if clientConnection(context.Background(), a).Value(clientAddressKey{}) != nil {
		t.Fatal("untrusted connection gained Funnel source")
	}
}
