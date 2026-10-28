package remote

import (
	"context"
	"crypto/tls"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"tailscale.com/ipn"
	"tailscale.com/tsnet"
)

func embeddedNode(home, name string, logf func(string, ...any)) (*tsnet.Server, error) {
	if name == "" {
		name = "bridge"
	}
	if len(name) > 63 || strings.Trim(name, "abcdefghijklmnopqrstuvwxyz0123456789-") != "" || strings.HasPrefix(name, "-") || strings.HasSuffix(name, "-") {
		return nil, fmt.Errorf("embedded name must be a DNS label")
	}
	dir := filepath.Join(home, "remote")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	if err := os.Chmod(dir, 0700); err != nil {
		return nil, err
	}
	return &tsnet.Server{Dir: dir, Hostname: name, UserLogf: logf, Logf: func(string, ...any) {}}, nil
}

type clientAddressKey struct{}

func clientConnection(ctx context.Context, conn net.Conn) context.Context {
	if secure, ok := conn.(*tls.Conn); ok {
		if funnel, ok := secure.NetConn().(*ipn.FunnelConn); ok {
			return context.WithValue(ctx, clientAddressKey{}, funnel.Src.String())
		}
	}
	return ctx
}

type embeddedServer struct{ *tsnet.Server }

func (s embeddedServer) LocalClient() (LocalClient, error) { return s.Server.LocalClient() }
func (s embeddedServer) ListenFunnel(network, address string) (net.Listener, error) {
	return s.Server.ListenFunnel(network, address)
}
