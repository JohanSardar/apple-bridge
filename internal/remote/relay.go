package remote

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
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
		endpoint = "https://ab.lexaai.tech"
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
	if cfg.Domain != "" {
		host = cfg.Domain
		for {
			missing := relay.MissingRecords(ctx, net.DefaultResolver, host, id.Credential.Hostname)
			if len(missing) == 0 {
				break
			}
			status.State = "pending"
			status.Message = "Missing DNS records: " + strings.Join(missing, "; ")
			r.publish(*status)
			select {
			case <-ctx.Done():
				return
			case <-time.After(5 * time.Second):
			}
		}
		p, e := relay.NewProof(ctx, http.DefaultClient, endpoint, id.Key, id.Credential, "domain:"+host)
		if e != nil {
			fail(e)
			return
		}
		if e = relayRequest(ctx, endpoint+"/domain", map[string]any{"proof": p, "domain": host}); e != nil {
			fail(e)
			return
		}
	}
	status.URL = "https://" + host
	r.publish(*status)
	go r.watchCT(ctx, id)
	manager := &certificateManager{home: r.home, identity: id, host: host}
	bounded, cancel = context.WithTimeout(ctx, 5*time.Minute)
	err = manager.ensure(bounded)
	cancel()
	if err != nil {
		fail(err)
		return
	}
	go func() {
		ticker := time.NewTicker(12 * time.Hour)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				bounded, cancel := context.WithTimeout(ctx, 5*time.Minute)
				err := manager.ensure(bounded)
				cancel()
				if ctx.Err() == nil {
					r.mu.Lock()
					if r.status.ID == cfg.ID {
						if err != nil {
							r.status.LastError = "Certificate renewal failed"
						} else if cert, e := manager.get(nil); e == nil {
							if leaf, e := x509.ParseCertificate(cert.Certificate[0]); e == nil {
								r.status.CertificateExpiry = leaf.NotAfter
							}
						}
						write(r.home, "status", r.status)
					}
					r.mu.Unlock()
				}
			}
		}
	}()
	listener := relay.ListenTunnel(ctx, endpoint, http.DefaultClient, id.Key, id.Credential)
	defer listener.Close()
	secure := tls.NewListener(listener, &tls.Config{MinVersion: tls.VersionTLS12, GetCertificate: manager.get})
	r.ready(ctx, status, secure, "", fail)
}

func (r *Remote) watchCT(ctx context.Context, id relayIdentity) {
	ticker := time.NewTicker(time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
		bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
		p, err := relay.NewProof(bounded, http.DefaultClient, id.Endpoint, id.Key, id.Credential, "ct")
		if err == nil {
			raw, _ := json.Marshal(p)
			req, _ := http.NewRequestWithContext(bounded, "GET", id.Endpoint+"/ct", nil)
			req.Header.Set("X-Bridge-Proof", base64.RawURLEncoding.EncodeToString(raw))
			resp, e := http.DefaultClient.Do(req)
			if e == nil {
				var result struct {
					Warning string `json:"warning"`
				}
				if resp.StatusCode == 200 && json.NewDecoder(io.LimitReader(resp.Body, 4096)).Decode(&result) == nil {
					r.mu.Lock()
					if ctx.Err() == nil && r.status.Backend == "relay" {
						r.status.CTWarning = result.Warning
					}
					write(r.home, "status", r.status)
					r.mu.Unlock()
				}
				resp.Body.Close()
			}
		}
		cancel()
	}
}
