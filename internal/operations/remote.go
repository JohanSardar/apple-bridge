package operations

import (
	"bytes"
	"context"
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
func (g *RemoteGateway) call(ctx context.Context, p trust.Principal, f forwarded, out any) error {
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
	if json.Unmarshal(raw, out) != nil {
		return contracts.ErrCorrupt
	}
	return nil
}
func (g *RemoteGateway) Submit(ctx context.Context, p trust.Principal, c contracts.Command) (contracts.CommandRecord, error) {
	var r contracts.CommandRecord
	if _, err := contracts.ValidateCommand(c); err != nil {
		return r, err
	}
	if !trust.Subset([]contracts.Grant{"reminders:write"}, p.Grants) {
		return r, contracts.ErrUnauthorized
	}
	err := g.call(ctx, p, forwarded{Method: "submit", Command: c}, &r)
	if errors.Is(err, contracts.ErrUnavailable) {
		status, lookupErr := g.Status(ctx, p, contracts.CommandQuery{ID: c.ID})
		if lookupErr == nil {
			return status, nil
		}
	}
	if err == nil && r.CommitSeq > 0 && g.Wait != nil {
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
func (g *LocalGateway) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		reply := func(result any, err error) {
			w.Header().Set("Content-Type", "application/json")
			if err != nil {
				typed := contracts.ErrUnavailable
				var e *contracts.Error
				if errors.As(err, &e) {
					typed = e
				}
				w.WriteHeader(400)
				json.NewEncoder(w).Encode(map[string]any{"error": typed})
				return
			}
			json.NewEncoder(w).Encode(result)
		}
		if r.Method != "POST" || r.TLS == nil || len(r.TLS.PeerCertificates) == 0 {
			reply(nil, contracts.ErrUnauthorized)
			return
		}
		b, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1024*1024))
		if err != nil {
			reply(nil, contracts.ErrInvalid)
			return
		}
		var f forwarded
		if contracts.DecodeInput(b, &f) != nil {
			reply(nil, contracts.ErrInvalid)
			return
		}
		hash, err := forwardingHash(f)
		if err != nil {
			reply(nil, err)
			return
		}
		id := f.Query.ID
		if f.Method == "submit" {
			id = f.Command.ID
		}
		p, err := g.Authority.VerifyDelegation(r.TLS.PeerCertificates[0], r.Header.Get("X-Apple-Bridge-Delegation"), id, hash)
		if err != nil {
			reply(nil, err)
			return
		}
		var result any
		switch f.Method {
		case "submit":
			result, err = g.Submit(r.Context(), p, f.Command)
		case "status":
			result, err = g.Status(r.Context(), p, f.Query)
		case "list":
			result, err = g.List(r.Context(), p, f.Query)
		default:
			err = contracts.ErrInvalid
		}
		reply(result, err)
	})
}
