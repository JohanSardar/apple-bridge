package trust

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (a *Authority) PinnedListener(address, fingerprint string) (net.Listener, error) {
	return tls.Listen("tcp", address, &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{a.Certificate}, ClientAuth: tls.RequireAnyClientCert, VerifyConnection: a.ClientTLS(fingerprint).VerifyConnection})
}

func (a *Authority) JoinListen(ctx context.Context, code, address string) error {
	c, err := DecodePairCode(code)
	if err != nil {
		return err
	}
	if c.Dial == "" {
		return contracts.ErrInvalid
	}
	existing, err := a.Enrollment()
	if err != nil {
		return err
	}
	if existing != nil {
		return contracts.ErrConflict
	}
	listener, err := a.PinnedListener(address, c.Fingerprint)
	if err != nil {
		return err
	}
	defer listener.Close()
	ctx, cancel := context.WithDeadline(ctx, c.ExpiresAt)
	defer cancel()
	done := make(chan struct{}, 1)
	var mu sync.Mutex
	var enrolled bool
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/pair/proof", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" {
			http.Error(w, "invalid", 400)
			return
		}
		var req JoinRequest
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&req) != nil || req.ID != c.ID {
			http.Error(w, "unauthorized", 401)
			return
		}
		json.NewEncoder(w).Encode(JoinRequest{ID: c.ID, Proof: PairProof(c.Secret, a.Certificate.Certificate[0])})
	})
	mux.HandleFunc("/v1/pair/finish", func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		var p Principal
		if r.Method != "POST" || json.NewDecoder(http.MaxBytesReader(w, r.Body, 16384)).Decode(&p) != nil || p.Kind != "replica" || p.ID == "" || p.Fingerprint != a.Fingerprint || p.Dial != c.Dial {
			http.Error(w, "unauthorized", 401)
			return
		}
		if !enrolled {
			if err := a.SaveEnrollment(Enrollment{Dial: c.Dial, ReplicaID: p.ID, Address: c.Address, Fingerprint: c.Fingerprint, Grants: p.Grants, Ceiling: p.Grants}); err != nil {
				http.Error(w, "unavailable", 503)
				return
			}
			enrolled = true
		}
		json.NewEncoder(w).Encode(struct{}{})
		select {
		case done <- struct{}{}:
		default:
		}
	})
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	defer server.Close()
	failures := make(chan error, 1)
	go func() { failures <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case err := <-failures:
		return err
	case <-done:
		shutdown, stop := context.WithTimeout(context.Background(), time.Second)
		defer stop()
		server.Shutdown(shutdown)
		return nil
	}
}

func (a *Authority) DialPairings(ctx context.Context) {
	active := map[string]bool{}
	completed := make(chan string, 32)
	tick := time.NewTicker(250 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case id := <-completed:
			delete(active, id)
		case <-tick.C:
			d, err := a.load()
			if err != nil {
				continue
			}
			for id, pair := range d.Pairings {
				if pair.Dial == "" || pair.Complete || time.Now().After(pair.ExpiresAt) || active[id] || (!pair.Used && pair.Attempts >= 5) {
					continue
				}
				active[id] = true
				go func(id string, pair pairing) {
					a.dialPair(ctx, id, pair)
					select {
					case completed <- id:
					case <-ctx.Done():
					}
				}(id, pair)
			}
		}
	}
}

func (a *Authority) dialPair(ctx context.Context, id string, pair pairing) {
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	config := &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{a.Certificate}, InsecureSkipVerify: true}
	transport := &http.Transport{TLSClientConfig: config, DisableKeepAlives: true}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	raw, _ := json.Marshal(JoinRequest{ID: id})
	request, err := http.NewRequestWithContext(bounded, "POST", "https://"+pair.Dial+"/v1/pair/proof", strings.NewReader(string(raw)))
	if err != nil {
		return
	}
	response, err := client.Do(request)
	if err != nil {
		return
	}
	defer response.Body.Close()
	if response.StatusCode != 200 || response.TLS == nil || len(response.TLS.PeerCertificates) != 1 {
		return
	}
	var proof JoinRequest
	if json.NewDecoder(http.MaxBytesReader(nil, response.Body, 4096)).Decode(&proof) != nil || proof.ID != id {
		return
	}
	cert := response.TLS.PeerCertificates[0]
	p, err := a.acceptPair(proof, cert, true)
	if err != nil {
		return
	}
	pinned := &http.Transport{TLSClientConfig: a.ClientTLS(p.Fingerprint)}
	defer pinned.CloseIdleConnections()
	client = &http.Client{Transport: pinned}
	raw, _ = json.Marshal(p)
	request, err = http.NewRequestWithContext(bounded, "POST", "https://"+pair.Dial+"/v1/pair/finish", strings.NewReader(string(raw)))
	if err != nil {
		return
	}
	response, err = client.Do(request)
	if err != nil {
		return
	}
	response.Body.Close()
	if response.StatusCode == 200 {
		a.update(func(d *credentials) error {
			current := d.Pairings[id]
			current.Complete = true
			current.Key = nil
			d.Pairings[id] = current
			return nil
		})
	}
}

func (a *Authority) DialReplicas() ([]Principal, error) {
	d, err := a.load()
	if err != nil {
		return nil, err
	}
	result := []Principal{}
	for _, p := range d.Principals {
		if p.Kind == "replica" && p.Dial != "" {
			result = append(result, p)
		}
	}
	return result, nil
}

func (a *Authority) ConfirmDial(id string) error {
	return a.update(func(d *credentials) error {
		for key, pair := range d.Pairings {
			if pair.Result != nil && pair.Result.ID == id && !pair.Complete {
				pair.Complete = true
				pair.Key = nil
				d.Pairings[key] = pair
			}
		}
		return nil
	})
}
