package remote

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
)

type relayIdentity struct {
	Key        ed25519.PrivateKey `json:"key"`
	Credential relay.Credential   `json:"credential"`
	Endpoint   string             `json:"endpoint"`
}

func relayEnroll(ctx context.Context, home, endpoint, zone string) (relayIdentity, error) {
	var id relayIdentity
	if err := read(home, "relay", &id); err == nil {
		if len(id.Key) != ed25519.PrivateKeySize || id.Endpoint != endpoint {
			return id, fmt.Errorf("relay identity configuration mismatch")
		}
		if id.Credential.Hostname != "" {
			return id, nil
		}
	} else if !os.IsNotExist(err) {
		return id, err
	} else {
		_, key, err := ed25519.GenerateKey(rand.Reader)
		if err != nil {
			return id, err
		}
		id = relayIdentity{Key: key, Endpoint: endpoint}
		if err = write(home, "relay", id); err != nil {
			return id, err
		}
	}
	p, err := relay.NewProof(ctx, http.DefaultClient, endpoint, id.Key, relay.Credential{}, "enroll")
	if err != nil {
		return id, err
	}
	in := relay.Enrollment{Nonce: p.Nonce, PublicKey: base64.RawURLEncoding.EncodeToString(id.Key.Public().(ed25519.PublicKey)), Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(id.Key, relay.EnrollmentMessage(zone, p.Nonce)))}
	raw, _ := json.Marshal(in)
	req, err := http.NewRequestWithContext(ctx, "POST", endpoint+"/enroll", bytes.NewReader(raw))
	if err != nil {
		return id, err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return id, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return id, fmt.Errorf("relay enrollment refused")
	}
	if err = json.NewDecoder(io.LimitReader(resp.Body, 4096)).Decode(&id.Credential); err != nil {
		return id, err
	}
	return id, write(home, "relay", id)
}

func (r *Remote) serveRelay(ctx context.Context, cfg Config, status *Status, fail func(error)) {
	endpoint := os.Getenv("APPLE_BRIDGE_RELAY_URL")
	if endpoint == "" {
		endpoint = "https://control.ab.lexaai.tech:8443"
	}
	if !strings.HasPrefix(endpoint, "https://") {
		fail(fmt.Errorf("relay control requires HTTPS"))
		return
	}
	zone := os.Getenv("APPLE_BRIDGE_RELAY_ZONE")
	if zone == "" {
		zone = "ab.lexaai.tech"
	}
	bounded, cancel := context.WithTimeout(ctx, 30*time.Second)
	id, err := relayEnroll(bounded, r.home, endpoint, zone)
	cancel()
	if err != nil {
		fail(err)
		return
	}
	host := id.Credential.Hostname
	status.URL = "https://" + host
	r.publish(*status)
	cert, err := tls.LoadX509KeyPair(filepath.Join(r.home, "remote", "relay-cert.pem"), filepath.Join(r.home, "remote", "relay-key.pem"))
	if err != nil {
		fail(err)
		return
	}
	listener := relay.ListenTunnel(ctx, endpoint, http.DefaultClient, id.Key, id.Credential)
	defer listener.Close()
	secure := tls.NewListener(listener, &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{cert}})
	r.ready(ctx, status, secure, "", fail)
}
