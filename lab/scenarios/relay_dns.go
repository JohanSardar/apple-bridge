package scenarios

import (
	"context"
	"crypto/ed25519"
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"golang.org/x/net/dns/dnsmessage"
)

type relayDNSFixture struct {
	mu          sync.Mutex
	name, value string
	calls       int
}

func (d *relayDNSFixture) TXT(_ context.Context, name, value string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.name = name
	d.value = value
	d.calls++
	return nil
}
func relayDNSProof(ctx context.Context, lab *relayLab, key ed25519.PrivateKey, a, b relay.Credential, add func(string, bool)) {
	dns := &relayDNSFixture{}
	lab.server.DNS = dns
	p, err := relay.NewProof(ctx, http.DefaultClient, lab.api.URL, key, a, "dns:fixture")
	if err != nil {
		add("proof", false)
		return
	}
	in := map[string]any{"proof": p, "value": "fixture"}
	add("signed_dns_own_host_only", relayPost(http.DefaultClient, lab.api.URL+"/dns", in, nil, "") == 204 && dns.name == "_acme-challenge."+a.Hostname && dns.value == "fixture")
	add("dns_proof_replay_refused", relayPost(http.DefaultClient, lab.api.URL+"/dns", in, nil, "") == 403)
	p, _ = relay.NewProof(ctx, http.DefaultClient, lab.api.URL, key, a, "dns:fixture")
	p.Hostname = b.Hostname
	in["proof"] = p
	add("cross_device_dns_refused", relayPost(http.DefaultClient, lab.api.URL+"/dns", in, nil, "") == 403 && dns.calls == 1)
	p, _ = relay.NewProof(ctx, http.DefaultClient, lab.api.URL, key, a, "dns:fixture")
	in["proof"] = p
	in["value"] = "changed"
	add("dns_value_bound_to_signature", relayPost(http.DefaultClient, lab.api.URL+"/dns", in, nil, "") == 403 && dns.calls == 1)
}
func relayOwnDomain(ctx context.Context, lab *relayLab, key ed25519.PrivateKey, a, b relay.Credential, add func(string, bool)) {
	host := a.Hostname
	conn, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		add("dns_listener", false)
		return
	}
	defer conn.Close()
	var mu sync.Mutex
	ready := false
	go func() {
		b := make([]byte, 4096)
		for {
			n, addr, err := conn.ReadFrom(b)
			if err != nil {
				return
			}
			var msg dnsmessage.Message
			if msg.Unpack(b[:n]) != nil {
				continue
			}
			msg.Header.Response = true
			msg.Header.Authoritative = true
			mu.Lock()
			enabled := ready
			mu.Unlock()
			if enabled {
				for _, q := range msg.Questions {
					if q.Type != dnsmessage.TypeCNAME {
						continue
					}
					target := host + "."
					if q.Name.String() == "_acme-challenge.mac.example.test." {
						target = "_acme-challenge." + target
					}
					name, e := dnsmessage.NewName(target)
					if e == nil {
						msg.Answers = append(msg.Answers, dnsmessage.Resource{Header: dnsmessage.ResourceHeader{Name: q.Name, Type: dnsmessage.TypeCNAME, Class: dnsmessage.ClassINET, TTL: 1}, Body: &dnsmessage.CNAMEResource{CNAME: name}})
					}
				}
			}
			raw, e := msg.Pack()
			if e == nil {
				conn.WriteTo(raw, addr)
			}
		}
	}()
	resolver := &net.Resolver{PreferGo: true, Dial: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "udp", conn.LocalAddr().String())
	}}
	bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	missing := relay.MissingRecords(bounded, resolver, "mac.example.test", host)
	add("exact_missing_cnames", len(missing) == 2 && missing[0] == "mac.example.test CNAME "+host+"." && missing[1] == "_acme-challenge.mac.example.test CNAME _acme-challenge."+host+".")
	lab.server.Resolver = resolver
	domain := "mac.example.test"
	claim := func(key ed25519.PrivateKey, credential relay.Credential) map[string]any {
		p, _ := relay.NewProof(ctx, http.DefaultClient, lab.api.URL, key, credential, "domain:"+domain)
		return map[string]any{"proof": p, "domain": domain}
	}
	var missingReply struct {
		Missing []string `json:"missing"`
	}
	add("domain_endpoint_requires_records", relayPost(http.DefaultClient, lab.api.URL+"/domain", claim(key, a), &missingReply, "") == 200 && len(missingReply.Missing) == 2)
	forged := claim(key, a)
	proof := forged["proof"].(relay.Proof)
	proof.Hostname = b.Hostname
	forged["proof"] = proof
	add("unauthorized_domain_claim_rejected", relayPost(http.DefaultClient, lab.api.URL+"/domain", forged, nil, "") == 403)
	mu.Lock()
	ready = true
	mu.Unlock()
	add("own_domain_records_ready", len(relay.MissingRecords(bounded, resolver, "mac.example.test", host)) == 0)
	signed := claim(key, a)
	add("signed_domain_registration", relayPost(http.DefaultClient, lab.api.URL+"/domain", signed, nil, "") == 204)
	add("domain_registration_replay_rejected", relayPost(http.DefaultClient, lab.api.URL+"/domain", signed, nil, "") == 403)
	origin := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte("alias-origin")) }))
	defer origin.Close()
	detach, err := lab.server.Attach(a.Hostname, a.Credential, relayTestTunnel{address: origin.Listener.Addr().String(), wire: &relayWire{}})
	add("alias_tunnel_attached", err == nil)
	if detach != nil {
		defer detach()
	}
	transport := &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", lab.address)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 3 * time.Second}
	response, routeErr := client.Get("https://" + domain + "/")
	body := ""
	if routeErr == nil {
		raw, _ := io.ReadAll(response.Body)
		body = string(raw)
		response.Body.Close()
	}
	add("custom_sni_routes_through_relay", routeErr == nil && body == "alias-origin")
	relayCTProof(ctx, lab, key, a, add)

}

type relayCTTransport struct {
	assigned string
	seen     map[string]bool
}

func (t relayCTTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	host := r.URL.Query().Get("q")
	status := 200
	body := "unexpected-certificate"
	if host != "" {
		t.seen[host] = true
		body = "[]"
		if host != t.assigned {
			status = 503
		}
		if host == t.assigned {
			raw, _ := json.Marshal([]map[string]any{{"id": 1, "name_value": host}})
			body = string(raw)
		}
	}
	return &http.Response{StatusCode: status, Header: http.Header{}, Body: io.NopCloser(strings.NewReader(body))}, nil
}
func relayCTProof(ctx context.Context, lab *relayLab, key ed25519.PrivateKey, a relay.Credential, add func(string, bool)) {
	seen := map[string]bool{}
	lab.server.CTClient = &http.Client{Transport: relayCTTransport{a.Hostname, seen}}
	stopped, cancel := context.WithCancel(ctx)
	cancel()
	lab.server.MonitorCT(stopped)
	add("ct_checks_assigned_and_custom_names", seen[a.Hostname] && seen["mac.example.test"])
	p, _ := relay.NewProof(ctx, http.DefaultClient, lab.api.URL, key, a, "ct")
	raw, _ := json.Marshal(p)
	req, _ := http.NewRequestWithContext(ctx, "GET", lab.api.URL+"/ct", nil)
	req.Header.Set("X-Bridge-Proof", base64.RawURLEncoding.EncodeToString(raw))
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		add("assigned_name_unexpected_certificate_warns", false)
		return
	}
	defer resp.Body.Close()
	var result struct {
		Warning string `json:"warning"`
	}
	json.NewDecoder(resp.Body).Decode(&result)
	add("assigned_name_unexpected_certificate_warns", strings.Contains(result.Warning, a.Hostname+": Unexpected certificate"))
	add("ct_aggregates_name_warnings", strings.Contains(result.Warning, "mac.example.test: Certificate Transparency check unavailable") && strings.Contains(result.Warning, a.Hostname+": Unexpected certificate"))
}
