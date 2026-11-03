//go:build lab

package runtime

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

func serveRelayLab(ctx context.Context, home string, handler http.Handler) error {
	raw, err := os.ReadFile(filepath.Join(home, "lab-relay.json"))
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	var config struct {
		Listen string `json:"listen"`
		Cert   string `json:"cert"`
		Key    string `json:"key"`
	}
	if err = json.Unmarshal(raw, &config); err != nil {
		return err
	}
	certificate, err := tls.LoadX509KeyPair(config.Cert, config.Key)
	if err != nil {
		return err
	}
	listener, err := tls.Listen("tcp", config.Listen, &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{certificate}, NextProtos: []string{"http/1.1"}})
	if err != nil {
		return err
	}
	if err = os.MkdirAll(filepath.Join(home, "run"), 0700); err != nil {
		listener.Close()
		return err
	}
	if err = os.WriteFile(filepath.Join(home, "run", "lab-relay-addr"), []byte(listener.Addr().String()), 0600); err != nil {
		listener.Close()
		return err
	}
	mux := http.NewServeMux()
	mux.Handle("/v1/feed", handler)
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 30 * time.Second}
	go server.Serve(listener)
	go func() {
		<-ctx.Done()
		server.Close()
	}()
	return nil
}
