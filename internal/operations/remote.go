package operations

import (
	"bytes"
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type RemoteGateway struct {
	Exchange  func(context.Context, string, json.RawMessage) (contracts.CommandResponse, error)
	Authority *trust.Authority
	Connected func() bool
	Wait      func(context.Context, contracts.Cursor) error
}
type forwarded struct {
	Method  string                 `json:"method"`
	Command contracts.Command      `json:"command"`
	Query   contracts.CommandQuery `json:"query"`
}

func forwardingHash(f forwarded) (string, error) {
	if f.Method == "submit" {
		return contracts.CommandHash(f.Command)
	}
	b, err := json.Marshal(f)
	return trust.Fingerprint(b), err
}
func (g *RemoteGateway) call(ctx context.Context, p trust.Principal, f forwarded, out any, lookupOnly ...*bool) error {
	if !g.Connected() {
		return contracts.ErrUnavailable
	}
	e, err := g.Authority.Enrollment()
	if err != nil || e == nil {
		return contracts.ErrUnavailable
	}
	hash, err := forwardingHash(f)
	if err != nil {
		return err
	}
	id := f.Query.ID
	if f.Method == "submit" {
		id = f.Command.ID
	}
	header, err := g.Authority.SignDelegation(p, id, hash)
	if err != nil {
		return err
	}
	b, err := json.Marshal(f)
	if err != nil {
		return err
	}
	if e.Dial != "" {
		if g.Exchange == nil {
			return contracts.ErrUnavailable
		}
		response, err := g.Exchange(ctx, header, b)
		if err != nil {
			return err
		}
		if response.Error != nil {
			return response.Error
		}
		if len(lookupOnly) > 0 {
			*lookupOnly[0] = response.LookupOnly
		}
		if json.Unmarshal(response.Result, out) != nil {
			return contracts.ErrCorrupt
		}
		return nil
	}
	req, err := http.NewRequestWithContext(ctx, "POST", "https://"+e.Address+"/v1/commands", bytes.NewReader(b))
	if err != nil {
		return err
	}
	req.Header.Set("X-Apple-Bridge-Delegation", header)
	transport := &http.Transport{TLSClientConfig: g.Authority.ClientTLS(e.Fingerprint), DialContext: (&net.Dialer{Timeout: 500 * time.Millisecond}).DialContext, TLSHandshakeTimeout: time.Second}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 20 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return contracts.ErrUnavailable
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 1024*1024))
	if err != nil {
		return contracts.ErrUnavailable
	}
	if resp.StatusCode != 200 {
		var envelope struct {
			Error *contracts.Error `json:"error"`
		}
		if json.Unmarshal(raw, &envelope) != nil || envelope.Error == nil {
			return contracts.ErrUnavailable
		}
		return envelope.Error
	}
	if len(lookupOnly) > 0 {
		*lookupOnly[0] = resp.Header.Get("X-Apple-Bridge-Command-Lookup-Only") == "true"
	}
	if json.Unmarshal(raw, out) != nil {
		return contracts.ErrCorrupt
	}
	return nil
}
func (g *RemoteGateway) Submit(ctx context.Context, p trust.Principal, c contracts.Command) (contracts.CommandRecord, error) {
	var r contracts.CommandRecord
	c.Origin = ""
	if p.Kind == "device" {
		c.Origin = "interactive"
	}
	if _, err := contracts.ValidateCommand(c); err != nil {
		return r, err
	}
	descriptor, ok := contracts.Descriptor(c.Op)
	if !ok {
		return r, contracts.ErrUnsupported
	}
	if !trust.Subset([]contracts.Grant{descriptor.Scope}, p.Grants) {
		return r, contracts.ErrUnauthorized
	}
	lookupOnly := false
	err := g.call(ctx, p, forwarded{Method: "submit", Command: c}, &r, &lookupOnly)
	if errors.Is(err, contracts.ErrUnavailable) {
		var status contracts.CommandRecord
		lookupErr := g.call(ctx, p, forwarded{Method: "status", Query: contracts.CommandQuery{ID: c.ID}}, &status, &lookupOnly)
		if lookupErr == nil {
			r = status
			err = nil
		}
	}
	if err == nil && !lookupOnly && r.CommitSeq > 0 && g.Wait != nil {
		waitctx, cancel := context.WithTimeout(ctx, 15*time.Second)
		defer cancel()
		if waitErr := g.Wait(waitctx, r.Cursor); waitErr != nil {
			return r, waitErr
		}
	}
	return r, err
}
func (g *RemoteGateway) Status(ctx context.Context, p trust.Principal, q contracts.CommandQuery) (contracts.CommandRecord, error) {
	var r contracts.CommandRecord
	err := g.call(ctx, p, forwarded{Method: "status", Query: q}, &r)
	return r, err
}
func (g *RemoteGateway) List(ctx context.Context, p trust.Principal, q contracts.CommandQuery) ([]contracts.CommandRecord, error) {
	r := []contracts.CommandRecord{}
	err := g.call(ctx, p, forwarded{Method: "list", Query: q}, &r)
	return r, err
}
func (g *LocalGateway) Forward(ctx context.Context, cert *x509.Certificate, header string, raw json.RawMessage) (contracts.CommandResponse, error) {
	response := contracts.CommandResponse{}
	var f forwarded
	var result any
	err := contracts.DecodeInput(raw, &f)
	if err == nil {
		var hash string
		hash, err = forwardingHash(f)
		if err == nil {
			id := f.Query.ID
			if f.Method == "submit" {
				id = f.Command.ID
			}
			var p trust.Principal
			p, err = g.Authority.VerifyDelegation(cert, header, id, hash)
			if err == nil {
				switch f.Method {
				case "submit":
					result, err = g.Submit(ctx, p, f.Command)
				case "status":
					result, err = g.Status(ctx, p, f.Query)
				case "list":
					result, err = g.List(ctx, p, f.Query)
				default:
					err = contracts.ErrInvalid
				}
			}
		}
	}
	if f.Method == "submit" && dropCommandResponse() {
		return response, contracts.ErrUnavailable
	}
	if err == nil {
		if command, ok := result.(contracts.CommandRecord); ok {
			era, eraErr := g.State.AdmissionEra(ctx)
			err = eraErr
			response.LookupOnly = command.AdmissionEra != era
		}
	}
	if err != nil {
		response.Error = contracts.ErrUnavailable
		var typed *contracts.Error
		if errors.As(err, &typed) {
			response.Error = typed
		}
		return response, nil
	}
	response.Result, err = json.Marshal(result)
	return response, err
}

func (g *LocalGateway) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.TLS == nil || len(r.TLS.PeerCertificates) != 1 {
			http.Error(w, "unauthorized", 401)
			return
		}
		raw, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1024*1024))
		if err != nil {
			http.Error(w, "invalid", 400)
			return
		}
		response, err := g.Forward(r.Context(), r.TLS.PeerCertificates[0], r.Header.Get("X-Apple-Bridge-Delegation"), raw)
		if err != nil {
			conn, _, hijackErr := http.NewResponseController(w).Hijack()
			if hijackErr == nil {
				conn.Close()
				return
			}
			http.Error(w, "unavailable", 503)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		if response.Error != nil {
			w.WriteHeader(400)
			json.NewEncoder(w).Encode(response)
			return
		}
		if response.LookupOnly {
			w.Header().Set("X-Apple-Bridge-Command-Lookup-Only", "true")
		}
		w.Write(response.Result)
	})
}
