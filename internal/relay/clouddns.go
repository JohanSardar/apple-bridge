package relay

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"

	"golang.org/x/oauth2"
	"time"
)

type CloudDNS struct {
	Client  *http.Client
	Project string
	Zone    string
	Suffix  string
	mu      sync.Mutex
}
type dnsRecord struct {
	Name string   `json:"name"`
	Type string   `json:"type"`
	TTL  int      `json:"ttl"`
	Data []string `json:"rrdatas"`
}

func NewCloudDNS(ctx context.Context, project, zone, suffix string) (*CloudDNS, error) {
	source := oauth2.ReuseTokenSource(nil, vmTokenSource{})
	return &CloudDNS{Client: oauth2.NewClient(ctx, source), Project: project, Zone: zone, Suffix: suffix}, nil
}

type vmTokenSource struct{}

func (vmTokenSource) Token() (*oauth2.Token, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, "GET", "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token", nil)
	req.Header.Set("Metadata-Flavor", "Google")
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	defer client.CloseIdleConnections()
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("VM service account token unavailable")
	}
	var token struct {
		AccessToken string `json:"access_token"`
		Expires     int    `json:"expires_in"`
	}
	if err = json.NewDecoder(io.LimitReader(resp.Body, 8192)).Decode(&token); err != nil {
		return nil, err
	}
	if token.AccessToken == "" || token.Expires <= 0 {
		return nil, fmt.Errorf("invalid VM token")
	}
	return &oauth2.Token{AccessToken: token.AccessToken, TokenType: "Bearer", Expiry: time.Now().Add(time.Duration(token.Expires) * time.Second)}, nil
}

func (d *CloudDNS) TXT(ctx context.Context, name, value string) error {
	if len(value) > 128 || strings.Trim(value, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_") != "" {
		return ErrDenied
	}
	if !strings.HasPrefix(name, "_acme-challenge.") || !strings.HasSuffix(name, "."+d.Suffix) || !validHost(name[len("_acme-challenge."):]) {
		return ErrDenied
	}
	d.mu.Lock()
	defer d.mu.Unlock()
	base := "https://dns.googleapis.com/dns/v1/projects/" + url.PathEscape(d.Project) + "/managedZones/" + url.PathEscape(d.Zone)
	name = strings.TrimSuffix(name, ".") + "."
	req, err := http.NewRequestWithContext(ctx, "GET", base+"/rrsets?name="+url.QueryEscape(name)+"&type=TXT", nil)
	if err != nil {
		return err
	}
	resp, err := d.Client.Do(req)
	if err != nil {
		return err
	}
	var records struct {
		Records []dnsRecord `json:"rrsets"`
	}
	err = json.NewDecoder(io.LimitReader(resp.Body, 65536)).Decode(&records)
	resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("Cloud DNS read refused (%d); VM service account requires roles/dns.admin", resp.StatusCode)
	}
	if err != nil {
		return err
	}
	change := map[string]any{}
	if len(records.Records) > 0 {
		change["deletions"] = records.Records
	}
	if value != "" {
		change["additions"] = []dnsRecord{{Name: name, Type: "TXT", TTL: 30, Data: []string{"\"" + value + "\""}}}
	}
	if len(change) == 0 {
		return nil
	}
	raw, _ := json.Marshal(change)
	req, err = http.NewRequestWithContext(ctx, "POST", base+"/changes", bytes.NewReader(raw))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err = d.Client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("Cloud DNS change refused (%d); VM service account requires roles/dns.admin", resp.StatusCode)
	}
	return nil
}
