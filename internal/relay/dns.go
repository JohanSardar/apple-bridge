package relay

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"strings"
)

type DNSProvider interface {
	TXT(context.Context, string, string) error
}

func (s *Server) dns(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Proof Proof  `json:"proof"`
		Value string `json:"value"`
	}
	if !decode(w, r, &in) {
		return
	}
	if len(in.Value) > 128 || strings.Trim(in.Value, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_") != "" || !s.Store.Verify(in.Proof, "dns:"+in.Value) {
		http.Error(w, "denied", 403)
		return
	}
	if s.DNS == nil || s.DNS.TXT(r.Context(), "_acme-challenge."+in.Proof.Hostname, in.Value) != nil {
		http.Error(w, "DNS unavailable", 503)
		return
	}
	w.WriteHeader(204)
}

func MissingRecords(ctx context.Context, resolver *net.Resolver, domain, host string) []string {
	var missing []string
	for _, record := range [][2]string{{domain, host}, {"_acme-challenge." + domain, "_acme-challenge." + host}} {
		got, err := resolver.LookupCNAME(ctx, record[0])
		if err != nil || !strings.EqualFold(strings.TrimSuffix(got, "."), record[1]) {
			missing = append(missing, record[0]+" CNAME "+record[1]+".")
		}
	}
	return missing
}
func (s *Server) domain(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Proof  Proof  `json:"proof"`
		Domain string `json:"domain"`
	}
	if !decode(w, r, &in) {
		return
	}
	if !validHost(in.Domain) || !s.Store.Verify(in.Proof, "domain:"+in.Domain) {
		http.Error(w, "denied", 403)
		return
	}
	resolver := s.Resolver
	if resolver == nil {
		resolver = net.DefaultResolver
	}
	if missing := MissingRecords(r.Context(), resolver, in.Domain, in.Proof.Hostname); len(missing) > 0 {
		reply(w, map[string]any{"missing": missing})
		return
	}
	if err := s.Store.setDomain(in.Proof.Hostname, in.Domain); err != nil {
		http.Error(w, "domain refused", 403)
		return
	}
	w.WriteHeader(204)
}
func (s *Store) setDomain(host, domain string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	for h, d := range s.devices {
		if (h == domain || d.Domain == domain) && h != host {
			return ErrDenied
		}
	}
	d, ok := s.devices[host]
	if !ok {
		return ErrDenied
	}
	old := d
	d.Domain = domain
	s.devices[host] = d
	if err := s.save(); err != nil {
		s.devices[host] = old
		return fmt.Errorf("domain storage: %w", err)
	}
	return nil
}
func (s *Store) canonical(host string) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.devices[host]; ok {
		return host
	}
	for h, d := range s.devices {
		if d.Domain == host {
			return h
		}
	}
	return host
}
