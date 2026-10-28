package scenarios

import (
	"context"
	"crypto/ed25519"
	"net"
	"net/http"
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
func relayOwnDomain(ctx context.Context, host string, add func(string, bool)) {
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
	mu.Lock()
	ready = true
	mu.Unlock()
	add("own_domain_records_ready", len(relay.MissingRecords(bounded, resolver, "mac.example.test", host)) == 0)
}
