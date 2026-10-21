package main

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"net/http"
	"net/url"
	"os"
	"strings"
)

func publicDial(ctx context.Context, in input, enrollment trust.Enrollment) (*websocket.Conn, error) {
	var proxy struct {
		URL string `json:"url"`
	}
	raw, err := os.ReadFile(in.ProxyReady)
	if err != nil {
		return nil, err
	}
	if err = json.Unmarshal(raw, &proxy); err != nil {
		return nil, err
	}
	u, err := url.Parse(proxy.URL)
	if err != nil || u.Scheme != "http" || u.Hostname() != "127.0.0.1" {
		return nil, fmt.Errorf("invalid public route")
	}
	transport := &http.Transport{Proxy: http.ProxyURL(u), TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS13}}
	defer transport.CloseIdleConnections()
	conn, _, err := websocket.Dial(ctx, strings.Replace(enrollment.Address, "https:", "wss:", 1)+"/v1/feed", &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
	return conn, err
}
