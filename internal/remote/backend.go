package remote

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net"
	"strings"
	"tailscale.com/client/local"
	"tailscale.com/ipn"
	"time"
)

func (r *Remote) serve(ctx context.Context, cfg Config) {
	status := Status{ID: cfg.ID, State: "starting", Backend: cfg.Backend}
	fail := func(err error) {
		if ctx.Err() == nil {
			status.State = "error"
			status.LastError = err.Error()
			r.publish(status)
		}
	}
	client := &local.Client{}
	probe, cancel := context.WithTimeout(ctx, 2*time.Second)
	system, err := systemStatus(probe, client)
	cancel()
	if cfg.Backend == "" {
		if err == nil && system != nil {
			cfg.Backend = "system"
		} else {
			cfg.Backend = "embedded"
		}
	}
	status.Backend = cfg.Backend
	r.publish(status)
	if cfg.Backend == "system" {
		if err = cleanupSystem(r.home, client); err != nil {
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
		status.State = "on"
		r.publish(status)
		r.serveListener(ctx, listener, proxyPath, fail)
		return
	}
	node, err := embeddedNode(r.home, cfg.Name, func(format string, args ...any) { r.message(cfg.ID, fmt.Sprintf(format, args...)) })
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
	st, err := node.Up(ctx)
	if err != nil {
		fail(err)
		return
	}
	for {
		if st.Self != nil {
			err = ipn.CheckFunnelAccess(443, st.Self)
		} else {
			err = fmt.Errorf("Tailscale node has no identity")
		}
		if err == nil {
			break
		}
		lc, clientErr := node.LocalClient()
		if clientErr != nil {
			fail(clientErr)
			return
		}
		r.message(cfg.ID, enablement(ctx, lc, err))
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Second):
		}
		st, err = lc.StatusWithoutPeers(ctx)
		if err != nil {
			fail(err)
			return
		}
	}
	listener, err := node.ListenFunnel("tcp", ":443")
	if err != nil {
		fail(err)
		return
	}
	defer listener.Close()
	status.URL = "https://" + strings.TrimSuffix(st.Self.DNSName, ".")
	status.State = "on"
	r.publish(status)
	r.serveListener(ctx, listener, "", fail)
}
