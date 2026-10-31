package remote

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net"
	"os"
	"strings"
	"tailscale.com/client/local"
)

func (r *Remote) serve(ctx context.Context, cfg Config) {
	status := Status{ID: cfg.ID, State: "starting", Backend: cfg.Backend, TLSTermination: "on this Mac"}
	fail := func(_ error) {
		if ctx.Err() == nil {
			fmt.Fprintln(os.Stderr, "remote setup failed; turn Remote off and try again")
			status.State = "error"
			status.LastError = "Remote setup failed. Turn Remote off and try again."
			r.publish(status)
		}
	}
	if cfg.Backend == "private" {
		r.servePrivate(ctx, cfg, &status, fail)
		return
	}
	if cfg.Backend == "relay" {
		r.serveRelay(ctx, cfg, &status, fail)
		return
	}
	client := &local.Client{}
	if cfg.Backend == "" {
		cfg.Backend = "embedded"
	}
	status.Backend = cfg.Backend
	r.publish(status)
	if cfg.Backend == "system" {
		if err := cleanupSystem(r.home, client); err != nil {
			fail(err)
			return
		}
		listener, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			fail(err)
			return
		}
		defer listener.Close()
		secret := make([]byte, 32)
		if _, err := rand.Read(secret); err != nil {
			fail(err)
			return
		}
		proxyPath := "/" + hex.EncodeToString(secret)
		entry, err := addSystem(ctx, r.home, client, "http://"+listener.Addr().String()+proxyPath)
		if err != nil {
			fail(err)
			return
		}
		defer func() {
			if err := cleanupSystem(r.home, client); err != nil {
				fail(err)
			}
		}()
		status.URL = "https://" + entry.Host
		if entry.Port != 443 {
			status.URL += fmt.Sprintf(":%d", entry.Port)
		}
		r.ready(ctx, &status, listener, proxyPath, fail)
		return
	}
	factory := r.options.Node
	if factory == nil {
		factory = func(home, name string) (EmbeddedNode, error) {
			node, err := embeddedNode(home, name, func(string, ...any) {})
			if err != nil {
				return nil, err
			}
			return embeddedServer{node}, nil
		}
	}
	node, err := factory(r.home, cfg.Name)
	if err != nil {
		fail(err)
		return
	}
	defer node.Close()
	closed := make(chan struct{})
	defer close(closed)
	go func() {
		select {
		case <-ctx.Done():
			node.Close()
		case <-closed:
		}
	}()
	clientEmbedded, err := node.LocalClient()
	if err != nil {
		fail(err)
		return
	}
	st, err := r.waitForFunnel(ctx, clientEmbedded, &status)
	if err != nil {
		fail(err)
		return
	}
	if ctx.Err() != nil {
		return
	}
	listener, err := node.ListenFunnel("tcp", ":443")
	if err != nil {
		fail(err)
		return
	}
	defer listener.Close()
	status.URL = "https://" + strings.TrimSuffix(st.Self.DNSName, ".")
	r.ready(ctx, &status, listener, "", fail)
}
