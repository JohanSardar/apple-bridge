package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"sync"
	"time"
)

func proxy(ctx context.Context, args []string) error {
	set := flag.NewFlagSet("proxy", flag.ContinueOnError)
	host := set.String("host", "", "public HTTPS URL")
	address := set.String("ip", "", "public relay address")
	ready := set.String("ready", "", "private readiness file")
	if err := set.Parse(args); err != nil {
		return err
	}
	u, err := url.Parse(*host)
	ip := net.ParseIP(*address)
	if err != nil || u.Scheme != "https" || u.Host == "" || ip == nil || ip.IsPrivate() || ip.IsLoopback() || *ready == "" {
		return fmt.Errorf("invalid public route")
	}
	port := u.Port()
	if port == "" {
		port = "443"
	}
	target := net.JoinHostPort(u.Hostname(), port)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	defer listener.Close()
	var mu sync.Mutex
	connections := 0
	publish := func() error {
		raw, _ := json.Marshal(map[string]any{"url": "http://" + listener.Addr().String(), "connections": connections})
		if err := os.WriteFile(*ready+".tmp", raw, 0600); err != nil {
			return err
		}
		return os.Rename(*ready+".tmp", *ready)
	}
	if err = publish(); err != nil {
		return err
	}
	server := &http.Server{ReadHeaderTimeout: 5 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "CONNECT" || r.Host != target {
			http.Error(w, "route refused", 403)
			return
		}
		up, err := (&net.Dialer{Timeout: 10 * time.Second}).DialContext(ctx, "tcp", net.JoinHostPort(ip.String(), port))
		if err != nil {
			http.Error(w, "relay unreachable", 502)
			return
		}
		defer up.Close()
		down, buffer, err := w.(http.Hijacker).Hijack()
		if err != nil {
			return
		}
		defer down.Close()
		buffer.WriteString("HTTP/1.1 200 Connection Established\r\n\r\n")
		buffer.Flush()
		mu.Lock()
		connections++
		publish()
		mu.Unlock()
		done := make(chan struct{})
		go func() { io.Copy(up, buffer); up.Close(); close(done) }()
		go func() {
			select {
			case <-ctx.Done():
				up.Close()
				down.Close()
			case <-done:
			}
		}()
		io.Copy(down, up)
	})}
	defer server.Close()
	errors := make(chan error, 1)
	go func() { errors <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
		return nil
	case err := <-errors:
		return err
	}
}
