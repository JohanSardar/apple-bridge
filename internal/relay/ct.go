package relay

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

func (s *Server) certificate(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Proof       Proof  `json:"proof"`
		Fingerprint string `json:"fingerprint"`
	}
	if !decode(w, r, &in) {
		return
	}
	if in.Fingerprint == "" || len(in.Fingerprint) != 64 || strings.Trim(in.Fingerprint, "0123456789abcdef") != "" || !s.Store.Verify(in.Proof, "certificate:"+in.Fingerprint) {
		http.Error(w, "denied", 403)
		return
	}
	s.Store.mu.Lock()
	defer s.Store.mu.Unlock()
	d := s.Store.devices[in.Proof.Hostname]
	for _, fingerprint := range d.Certificates {
		if fingerprint == in.Fingerprint {
			w.WriteHeader(204)
			return
		}
	}
	if len(d.Certificates) >= 128 {
		http.Error(w, "certificate history full", 409)
		return
	}
	old := d
	d.Certificates = append(append([]string{}, d.Certificates...), in.Fingerprint)
	s.Store.devices[in.Proof.Hostname] = d
	if s.Store.save() != nil {
		s.Store.devices[in.Proof.Hostname] = old
		http.Error(w, "storage unavailable", 503)
		return
	}
	w.WriteHeader(204)
}
func (s *Server) ct(w http.ResponseWriter, r *http.Request) {
	var p Proof
	raw, err := base64.RawURLEncoding.DecodeString(r.Header.Get("X-Bridge-Proof"))
	if err != nil || len(raw) > 4096 || json.Unmarshal(raw, &p) != nil || !s.Store.Verify(p, "ct") {
		http.Error(w, "denied", 403)
		return
	}
	s.mu.Lock()
	warning := s.ctWarnings[p.Hostname]
	s.mu.Unlock()
	reply(w, map[string]string{"warning": warning})
}
func (s *Server) MonitorCT(ctx context.Context) {
	ticker := time.NewTicker(time.Hour)
	defer ticker.Stop()
	for {
		s.Store.mu.Lock()
		devices := make([]Device, 0, len(s.Store.devices))
		for _, d := range s.Store.devices {
			if !d.Revoked {
				devices = append(devices, d)
			}
		}
		s.Store.mu.Unlock()
		for _, d := range devices {
			hosts := []string{d.Hostname}
			if d.Domain != "" && d.Domain != d.Hostname {
				hosts = append(hosts, d.Domain)
			}
			var warnings []string
			for _, host := range hosts {
				bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
				fingerprints, err := ctFingerprints(bounded, host, s.CTClient)
				cancel()
				if current, ok := s.Store.Device(d.Hostname); ok {
					d = current
				}
				warning := ""
				if err != nil {
					warning = "Certificate Transparency check unavailable"
				} else {
					for _, fingerprint := range fingerprints {
						known := false
						for _, expected := range d.Certificates {
							if fingerprint == expected {
								known = true
								break
							}
						}
						if !known {
							warning = "Unexpected certificate found in Certificate Transparency"
							break
						}
					}
				}
				if warning != "" {
					warnings = append(warnings, host+": "+warning)
				}
			}
			s.mu.Lock()
			s.ctWarnings[d.Hostname] = strings.Join(warnings, "; ")
			s.mu.Unlock()
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}
func ctFingerprints(ctx context.Context, host string, client *http.Client) ([]string, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", "https://crt.sh/?q="+url.QueryEscape(host)+"&output=json", nil)
	if err != nil {
		return nil, err
	}
	if client == nil {
		client = &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("CT unavailable")
	}
	var rows []struct {
		ID    int64  `json:"id"`
		Names string `json:"name_value"`
	}
	if err = json.NewDecoder(io.LimitReader(resp.Body, 2<<20)).Decode(&rows); err != nil {
		return nil, err
	}
	var fingerprints []string
	for _, row := range rows {
		for _, name := range strings.Split(row.Names, "\n") {
			if strings.EqualFold(name, host) {
				req, e := http.NewRequestWithContext(ctx, "GET", "https://crt.sh/?d="+strconv.FormatInt(row.ID, 10), nil)
				if e != nil {
					return nil, e
				}
				response, e := client.Do(req)
				if e != nil {
					return nil, e
				}
				data, e := io.ReadAll(io.LimitReader(response.Body, 65537))
				response.Body.Close()
				if e != nil || response.StatusCode != 200 || len(data) > 65536 {
					return nil, fmt.Errorf("CT certificate unavailable")
				}
				if block, _ := pem.Decode(data); block != nil {
					data = block.Bytes
				}
				fingerprint := sha256.Sum256(data)
				fingerprints = append(fingerprints, hex.EncodeToString(fingerprint[:]))
				break
			}
		}
	}
	return fingerprints, nil
}
