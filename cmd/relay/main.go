package main

import (
	"context"
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func run() error {
	controlACME := flag.Bool("control-acme", false, "issue and renew trusted control TLS using Cloud DNS")
	addr := flag.String("addr", ":443", "TLS passthrough address")
	controlHost := flag.String("control-host", "", "route control TLS by SNI on the public port")
	control := flag.String("control-addr", "127.0.0.1:8443", "enrollment API address")
	zone := flag.String("zone", "", "device hostname zone")
	state := flag.String("state", "relay-state.json", "private persistent state path")
	cert := flag.String("control-cert", "", "control API TLS certificate")
	key := flag.String("control-key", "", "control API TLS private key")
	connections := flag.Int("connections", 32, "concurrent connections per device")
	rate := flag.Int64("bytes-per-second", 1048576, "combined bytes per second per device")
	monthly := flag.Int64("monthly-bytes", 10737418240, "combined monthly bytes per device")
	dnsProject := flag.String("dns-project", "", "Google Cloud DNS project")
	dnsZone := flag.String("dns-zone", "", "Google Cloud DNS managed zone")
	idle := flag.Duration("idle-timeout", 2*time.Minute, "maximum inactivity across both stream directions")
	flag.Parse()
	if *idle <= 0 {
		return fmt.Errorf("idle-timeout must be positive")
	}
	store, err := relay.Open(*state, *zone, relay.Limits{Connections: *connections, BytesPerSecond: *rate, MonthlyBytes: *monthly})
	if err != nil {
		return err
	}
	if !*controlACME && (*cert == "" || *key == "") {
		return fmt.Errorf("control-cert and control-key required")
	}
	server := relay.New(store)
	if *dnsProject != "" {
		server.DNS, err = relay.NewCloudDNS(context.Background(), *dnsProject, *dnsZone, *zone)
		if err != nil {
			return err
		}
	}
	server.IdleTimeout = *idle
	defer server.Close()
	listener, err := net.Listen("tcp", *addr)
	if err != nil {
		return err
	}
	defer listener.Close()
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	go server.MonitorCT(ctx)
	api := &http.Server{Addr: *control, Handler: server.Handler(os.Getenv("APPLE_BRIDGE_RELAY_ADMIN")), ReadHeaderTimeout: 5 * time.Second, ReadTimeout: 10 * time.Second, WriteTimeout: 6 * time.Minute, IdleTimeout: 30 * time.Second, MaxHeaderBytes: 8192}
	if *controlACME {
		api.TLSConfig, err = remote.ControlTLSConfig(ctx, filepath.Join(filepath.Dir(*state), "control-tls"), *controlHost, server.DNS, remote.CertificateOptions{})
		if err != nil {
			return err
		}
		*cert, *key = "", ""
	}
	result := make(chan error, 2)
	if *controlHost != "" {
		controlListener := relay.NewControlListener(ctx)
		defer controlListener.Close()
		server.ControlHost = *controlHost
		server.Control = controlListener
		go func() { result <- api.ServeTLS(controlListener, *cert, *key) }()
	} else {
		go func() { result <- api.ListenAndServeTLS(*cert, *key) }()
	}
	go func() { result <- server.Serve(ctx, listener) }()
	select {
	case err = <-result:
	case <-ctx.Done():
	}
	cancel()
	api.Close()
	return err
}
