package main

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func goGateway(home string) error {
	a, err := trust.Open(home)
	if err != nil {
		return err
	}
	s, err := state.Open(home)
	if err != nil {
		return err
	}
	defer s.Close()
	b, err := blobs.Open(home, blobs.Config{})
	if err != nil {
		return err
	}
	defer b.Close()
	digest, _, err := b.Stage(context.Background(), strings.NewReader("fixture"), blobs.Expect{Size: 7})
	if err != nil {
		return err
	}
	_ = digest
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	for range 2 {
		code, err := a.Pair(l.Addr().String(), []contracts.Grant{"reminders:read", "messages:write"})
		if err != nil {
			return err
		}
		fmt.Println(code)
	}
	gateway := &operations.LocalGateway{Authority: a, Uploads: b}
	server := &replication.Server{Store: s, Authority: a, Blobs: b, UploadControl: gateway.UploadControl, UploadChunk: gateway.UploadChunk, UploadClosed: gateway.UploadClosed}
	mux := http.NewServeMux()
	mux.Handle("/v1/pair", a.PairHandler())
	mux.Handle("/v1/feed", server.Handler())
	go func() {
		for {
			if _, err := os.Stat(filepath.Join(home, "advance")); err == nil {
				break
			}
			time.Sleep(10 * time.Millisecond)
		}
		var capture contracts.Capture
		raw := fmt.Sprintf(`{"source":"reminders","generation":"fixture","checkpoint":"2","predecessor":"1","observations":[{"kind":"upsert","collection":"reminders.list","native_key":"fixture-list","observed_at":"2026-01-02T00:00:00Z","data":{"title":"fixture-2","blob":{"digest":%q,"size":7}}}]}`, trust.Fingerprint([]byte("fixture")))
		if err := json.Unmarshal([]byte(raw), &capture); err != nil {
			panic(err)
		}
		if _, err := s.CommitCapture(context.Background(), capture); err != nil {
			panic(err)
		}
	}()
	return (&http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}).Serve(tls.NewListener(l, &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{a.Certificate}, ClientAuth: tls.RequireAnyClientCert}))
}

func copySeed(seed, home string) error {
	if err := os.MkdirAll(home, 0700); err != nil {
		return err
	}
	for _, name := range []string{"state.db", "ledger.db"} {
		data, err := os.ReadFile(filepath.Join(seed, name))
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(home, name), data, 0600); err != nil {
			return err
		}
	}
	return nil
}
