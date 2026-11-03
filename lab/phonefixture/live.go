package phonefixture

import (
	"context"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

type LivePushSink struct {
	server  *relay.Server
	packets chan simapple.IPhonePacket
	relay   *httptest.Server
	apns    *httptest.Server
	mu      sync.Mutex
	events  []string
	valid   bool
}

func LiveSink(g *Gateway) (*LivePushSink, error) {
	s := &LivePushSink{valid: true, packets: make(chan simapple.IPhonePacket, 32)}
	s.apns = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			APS map[string]json.RawMessage `json:"aps"`
		}
		raw, _ := io.ReadAll(r.Body)
		var event string
		s.mu.Lock()
		defer s.mu.Unlock()
		s.valid = s.valid && json.Unmarshal(raw, &body) == nil && json.Unmarshal(body.APS["event"], &event) == nil
		token := strings.Repeat("b", 64)
		if event == "start" {
			token = strings.Repeat("a", 64)
			s.valid = s.valid && string(body.APS["attributes-type"]) == `"AgentTaskAttributes"`
		}
		s.valid = s.valid && r.URL.Path == "/3/device/"+token && r.Header.Get("apns-push-type") == "liveactivity" && r.Header.Get("apns-topic") == "com.johansardar.applebridge.ios.push-type.liveactivity" && r.Header.Get("apns-priority") == "10" && len(body.APS["content-state"]) > 0
		s.events = append(s.events, event)
		w.WriteHeader(200)
	}))
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		s.Close()
		return nil, err
	}
	apns := &relay.APNs{Key: key, KeyID: "ab-test", Team: "ab-test", Client: s.apns.Client(), Endpoint: s.apns.URL}
	store, err := relay.Open(filepath.Join(filepath.Dir(g.Home), "relay.json"), "example.test", relay.Limits{Connections: 4, BytesPerSecond: 65536, MonthlyBytes: 1048576})
	if err != nil {
		s.Close()
		return nil, err
	}
	_, signing, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		s.Close()
		return nil, err
	}
	nonce, err := store.Challenge()
	if err != nil {
		s.Close()
		return nil, err
	}
	credential, err := store.Enroll(relay.Enrollment{Nonce: nonce, PublicKey: base64.RawURLEncoding.EncodeToString(signing.Public().(ed25519.PublicKey)), Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(signing, relay.EnrollmentMessage("example.test", nonce)))})
	if err != nil {
		s.Close()
		return nil, err
	}
	s.server = relay.New(store)
	s.server.IPhonePush = func(ctx context.Context, token string, raw json.RawMessage, expires time.Time) error {
		var packet simapple.IPhonePacket
		if err := json.Unmarshal(raw, &packet); err != nil {
			return err
		}
		if len(packet.LiveActivity) == 0 {
			select {
			case s.packets <- packet:
				return nil
			case <-ctx.Done():
				return ctx.Err()
			}
		}
		return apns.Send(ctx, token, raw, expires)
	}
	certPEM, keyPEM, err := fixtureCertificate()
	if err != nil {
		s.Close()
		return nil, err
	}
	certificate, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		s.Close()
		return nil, err
	}
	s.relay = httptest.NewUnstartedServer(s.server.Handler(""))
	s.relay.TLS = &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{certificate}}
	s.relay.StartTLS()
	certPath := filepath.Join(filepath.Dir(g.Home), "fixture.pem")
	file, err := os.OpenFile(certPath, os.O_APPEND|os.O_WRONLY, 0600)
	if err == nil {
		_, err = file.Write(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: s.relay.Certificate().Raw}))
		file.Close()
	}
	if err == nil {
		err = os.MkdirAll(filepath.Join(g.Home, "remote"), 0700)
	}
	raw, _ := json.Marshal(map[string]any{"key": signing, "endpoint": s.relay.URL, "credential": credential})
	if err == nil {
		err = os.WriteFile(filepath.Join(g.Home, "remote", "relay.json"), raw, 0600)
	}
	if err == nil {
		err = g.Restart()
	}
	if err != nil {
		s.Close()
		return nil, err
	}
	return s, nil
}
func (s *LivePushSink) Wait(expected string) bool {
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		s.mu.Lock()
		events, valid := strings.Join(s.events, ","), s.valid
		s.mu.Unlock()
		if events == expected {
			time.Sleep(150 * time.Millisecond)
			s.mu.Lock()
			defer s.mu.Unlock()
			return s.valid && strings.Join(s.events, ",") == expected
		}
		if !valid {
			return false
		}
		time.Sleep(20 * time.Millisecond)
	}
	return false
}
func (s *LivePushSink) Close() {
	if s.server != nil {
		s.server.Close()
	}
	if s.relay != nil {
		s.relay.Close()
	}
	if s.apns != nil {
		s.apns.Close()
	}
}

func (s *LivePushSink) Detail() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return fmt.Sprintf("events=%v valid=%v", s.events, s.valid)
}

func (s *LivePushSink) Next() (simapple.IPhonePacket, error) {
	select {
	case packet := <-s.packets:
		return packet, nil
	case <-time.After(5 * time.Second):
		return simapple.IPhonePacket{}, fmt.Errorf("push timeout")
	}
}
