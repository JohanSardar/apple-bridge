package relay

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
)

type APNs struct {
	Key      *ecdsa.PrivateKey
	KeyID    string
	Team     string
	Sandbox  bool
	Client   *http.Client
	Endpoint string
}

func LoadAPNs(path, keyID, team string, sandbox bool) (*APNs, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	block, _ := pem.Decode(raw)
	if block == nil {
		return nil, fmt.Errorf("invalid APNs key")
	}
	key, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, err
	}
	signing, ok := key.(*ecdsa.PrivateKey)
	if !ok || signing.Curve != elliptic.P256() {
		return nil, fmt.Errorf("APNs key must use P-256")
	}
	return &APNs{Key: signing, KeyID: keyID, Team: team, Sandbox: sandbox, Client: &http.Client{Timeout: 10 * time.Second}}, nil
}

func (a *APNs) token() (string, error) {
	header, _ := json.Marshal(map[string]string{"alg": "ES256", "kid": a.KeyID})
	claims, _ := json.Marshal(map[string]any{"iss": a.Team, "iat": time.Now().Unix()})
	encoded := base64.RawURLEncoding.EncodeToString(header) + "." + base64.RawURLEncoding.EncodeToString(claims)
	hash := sha256.Sum256([]byte(encoded))
	r, s, err := ecdsa.Sign(rand.Reader, a.Key, hash[:])
	if err != nil {
		return "", err
	}
	signature := make([]byte, 64)
	r.FillBytes(signature[:32])
	s.FillBytes(signature[32:])
	return encoded + "." + base64.RawURLEncoding.EncodeToString(signature), nil
}

func (a *APNs) Send(ctx context.Context, device string, packet json.RawMessage, expires time.Time) error {
	if len(device) < 32 || len(device) > 512 || strings.Trim(device, "0123456789abcdef") != "" {
		return ErrDenied
	}
	body, err := json.Marshal(map[string]any{"aps": map[string]int{"content-available": 1}, "bridge": packet})
	if err != nil || len(body) > 4096 {
		return ErrDenied
	}
	live := false
	var envelope struct {
		Live json.RawMessage `json:"live_activity"`
	}
	if json.Unmarshal(packet, &envelope) == nil && len(envelope.Live) > 0 {
		body, err = json.Marshal(map[string]any{"aps": envelope.Live})
		if err != nil || len(body) > 4096 {
			return ErrDenied
		}
		live = true
	}
	endpoint := a.Endpoint
	if endpoint == "" {
		endpoint = "https://api.push.apple.com"
		if a.Sandbox {
			endpoint = "https://api.sandbox.push.apple.com"
		}
	}
	token, err := a.token()
	if err != nil {
		return err
	}
	request, err := http.NewRequestWithContext(ctx, "POST", endpoint+"/3/device/"+device, bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("authorization", "bearer "+token)
	request.Header.Set("apns-topic", "com.johansardar.applebridge.ios")
	request.Header.Set("apns-push-type", "background")
	request.Header.Set("apns-priority", "5")
	if live {
		request.Header.Set("apns-topic", "com.johansardar.applebridge.ios.push-type.liveactivity")
		request.Header.Set("apns-push-type", "liveactivity")
		request.Header.Set("apns-priority", "10")
	}
	request.Header.Set("apns-expiration", strconv.FormatInt(expires.Unix(), 10))
	response, err := a.Client.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	io.Copy(io.Discard, io.LimitReader(response.Body, 4096))
	if response.StatusCode != 200 {
		return fmt.Errorf("APNs rejected push: %d", response.StatusCode)
	}
	return nil
}
