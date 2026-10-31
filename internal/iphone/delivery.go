package iphone

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Push func(context.Context, string, Packet, time.Time) error

type ConnectorApproval struct {
	ID      string
	Code    string
	Name    string
	Scopes  []contracts.Grant
	Expires time.Time
}

type Delivery struct {
	ConnectorPending func() []ConnectorApproval
	ConnectorDecide  func(string, string, bool) error
	Blobs            *blobs.Store
	State            *state.Store
	Authority        *trust.Authority
	Push             Push
	Authorize        func(context.Context, contracts.CommandRecord) error
	Decide           func(context.Context, string, string, bool) (contracts.CommandRecord, error)
	mu               sync.Mutex
	wake             chan struct{}
}

type Job struct {
	Command contracts.Command `json:"command"`
	Hash    string            `json:"request_hash"`
	Expires int64             `json:"expires"`
}

type Result struct {
	ID    string                 `json:"command_id"`
	Hash  string                 `json:"request_hash"`
	State contracts.CommandState `json:"state"`
	Data  json.RawMessage        `json:"data"`
}

func NewDelivery(store *state.Store, authority *trust.Authority) *Delivery {
	return &Delivery{State: store, Authority: authority, wake: make(chan struct{}, 1)}
}
func (d *Delivery) Wake() {
	select {
	case d.wake <- struct{}{}:
	default:
	}
}

func (d *Delivery) transition(ctx context.Context, r contracts.CommandRecord, to contracts.CommandState, disposition contracts.Disposition, evidence json.RawMessage) (contracts.CommandRecord, error) {
	from := r.State
	r.State = to
	r.Disposition = disposition
	r.Evidence = evidence
	return d.State.TransitionCommand(ctx, state.CommandTransition{From: from, Record: r})
}

func targetOf(r contracts.CommandRecord) string {
	var target contracts.IPhoneTarget
	json.Unmarshal(r.Command.Args, &target)
	return target.DeviceID
}

func (d *Delivery) packets(ctx context.Context, p trust.Principal, dispatch bool) ([]Packet, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	rows, err := d.State.Commands(ctx)
	if err != nil {
		return nil, err
	}
	liveRows := []contracts.CommandRecord{}
	for _, row := range rows {
		if IsLiveCommand(row.Command.Op) {
			liveRows = append(liveRows, row)
		}
	}
	sort.Slice(liveRows, func(i, j int) bool {
		if liveRows[i].CreatedAt.Equal(liveRows[j].CreatedAt) {
			return liveRows[i].Command.ID < liveRows[j].Command.ID
		}
		return liveRows[i].CreatedAt.Before(liveRows[j].CreatedAt)
	})
	nextLive := 0
	for i := range rows {
		if IsLiveCommand(rows[i].Command.Op) {
			rows[i] = liveRows[nextLive]
			nextLive++
		}
	}
	packets := []Packet{}
	if d.ConnectorPending != nil {
		for _, approval := range d.ConnectorPending() {
			args, _ := json.Marshal(map[string]string{"device_id": p.ID, "body": fmt.Sprintf("%s\nCode %s\nRequested data: %s", approval.Name, approval.Code, approval.Scopes)})
			plain, _ := json.Marshal(Job{Command: contracts.Command{ID: approval.ID, Op: "bridge.approval", Args: args}, Hash: approval.Code, Expires: approval.Expires.Unix()})
			packet, err := Seal(p.IPhoneEncryptionKey, p.ID, plain, d.Authority.SignIPhone)
			if err != nil {
				return nil, err
			}
			packets = append(packets, packet)
		}
	}
	for _, r := range rows {
		if r.State == contracts.CommandAwaitingApproval && time.Now().Before(r.ExpiresAt) {
			args, _ := json.Marshal(map[string]string{"device_id": p.ID, "title": "Apple Bridge approval", "body": r.Command.Op + "\nAgent: " + r.Principal + "\n" + string(r.Command.Args)})
			plain, _ := json.Marshal(Job{Command: contracts.Command{ID: r.Command.ID, Op: "bridge.approval", Args: args}, Hash: r.RequestHash, Expires: r.ExpiresAt.Unix()})
			packet, sealErr := Seal(p.IPhoneEncryptionKey, p.ID, plain, d.Authority.SignIPhone)
			if sealErr != nil {
				return nil, sealErr
			}
			packets = append(packets, packet)
			continue
		}
		if r.Source != contracts.IPhone || targetOf(r) != p.ID {
			continue
		}
		if r.State != contracts.CommandAdmitted && r.State != contracts.CommandPrepared && r.State != contracts.CommandDispatched {
			continue
		}
		if !time.Now().Before(r.ExpiresAt) {
			to, disposition := contracts.CommandExpired, contracts.DispositionExpired
			if r.State == contracts.CommandDispatched {
				to, disposition = contracts.CommandIndeterminate, contracts.DispositionIndeterminate
			}
			if _, err = d.transition(ctx, r, to, disposition, nil); err != nil {
				return nil, err
			}
			continue
		}
		var authorization error
		if d.Authorize == nil {
			authorization = contracts.ErrUnauthorized
		} else {
			authorization = d.Authorize(ctx, r)
		}
		if errors.Is(authorization, contracts.ErrUnavailable) {
			continue
		}
		if authorization != nil {
			disposition := contracts.DispositionRejectedBeforeDispatch
			to := contracts.CommandRejected
			if r.State == contracts.CommandDispatched {
				to, disposition = contracts.CommandIndeterminate, contracts.DispositionIndeterminate
			}
			if _, err = d.transition(ctx, r, to, disposition, nil); err != nil {
				return nil, err
			}
			continue
		}
		if !dispatch {
			continue
		}
		if IsLiveCommand(r.Command.Op) {
			if r.State == contracts.CommandDispatched || d.Push == nil {
				continue
			}
			token, aps, liveErr := LivePayload(p, r.Command, time.Now())
			if liveErr != nil {
				continue
			}
			if r.State == contracts.CommandAdmitted {
				r, err = d.transition(ctx, r, contracts.CommandPrepared, "", nil)
				if err != nil {
					return nil, err
				}
			}
			r, err = d.transition(ctx, r, contracts.CommandDispatched, "", nil)
			if err != nil {
				return nil, err
			}
			plain, _ := json.Marshal(Job{Command: r.Command, Hash: r.RequestHash, Expires: r.ExpiresAt.Unix()})
			packet, sealErr := Seal(p.IPhoneEncryptionKey, p.ID, plain, d.Authority.SignIPhone)
			if sealErr != nil {
				return nil, sealErr
			}
			packet.LiveActivity = aps
			bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
			d.Push(bounded, token, packet, time.Now().Add(time.Minute))
			cancel()
			continue
		}
		if r.State == contracts.CommandAdmitted {
			r, err = d.transition(ctx, r, contracts.CommandPrepared, "", nil)
			if err != nil {
				return nil, err
			}
		}
		if r.State == contracts.CommandPrepared {
			r, err = d.transition(ctx, r, contracts.CommandDispatched, "", nil)
			if err != nil {
				return nil, err
			}
		}
		plain, err := json.Marshal(Job{Command: r.Command, Hash: r.RequestHash, Expires: r.ExpiresAt.Unix()})
		if err != nil {
			return nil, err
		}
		packet, err := Seal(p.IPhoneEncryptionKey, p.ID, plain, d.Authority.SignIPhone)
		if err != nil {
			return nil, err
		}
		packets = append(packets, packet)
		if len(packets) >= 32 {
			break
		}
	}
	return packets, nil
}

func (d *Delivery) Complete(ctx context.Context, p trust.Principal, request trust.IPhoneRequest) (contracts.CommandRecord, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	var result Result
	if contracts.DecodeInput(request.Payload, &result) != nil {
		return contracts.CommandRecord{}, contracts.ErrInvalid
	}
	r, err := d.State.Command(ctx, result.ID)
	if err != nil {
		return r, err
	}
	if r.Source != contracts.IPhone || targetOf(r) != p.ID || r.RequestHash != result.Hash {
		return r, contracts.ErrUnauthorized
	}
	if contracts.IPhoneLocationCommand(r.Command.Op) && (result.State == contracts.CommandObserved || string(result.Data) != "null") && contracts.ValidateIPhoneLocationSummary(result.Data) != nil {
		return r, contracts.ErrInvalid
	}
	if r.State != contracts.CommandDispatched && r.State != contracts.CommandIndeterminate {
		return r, nil
	}
	dispositions := map[contracts.CommandState]contracts.Disposition{contracts.CommandObserved: contracts.DispositionLocallyObserved, contracts.CommandFailed: contracts.DispositionFailedNoEffect, contracts.CommandIndeterminate: contracts.DispositionIndeterminate, contracts.CommandRejected: contracts.DispositionRejectedBeforeDispatch}
	disposition, ok := dispositions[result.State]
	if !ok {
		return r, contracts.ErrInvalid
	}
	if r.State == result.State {
		return r, nil
	}
	evidence, err := json.Marshal(struct {
		trust.IPhoneRequest
		SigningKey []byte `json:"signing_key"`
	}{request, p.IPhoneSigningKey})
	if err != nil {
		return r, err
	}
	return d.transition(ctx, r, result.State, disposition, evidence)
}

func (d *Delivery) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		limit := int64(32768)
		if r.URL.Path == "/v1/iphone/capture" {
			limit = 24 << 20
		}
		if r.URL.Path == "/v1/iphone/health" {
			limit = 16 << 20
		}
		var request trust.IPhoneRequest
		if r.Method != "POST" || json.NewDecoder(http.MaxBytesReader(w, r.Body, limit)).Decode(&request) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		p, err := d.Authority.VerifyIPhone(r.URL.Path, request)
		if err != nil {
			http.Error(w, "unauthorized", 401)
			return
		}
		var result any
		switch r.URL.Path {
		case "/v1/iphone/capture":
			result, err = d.Capture(r.Context(), p, request.Payload)
		case "/v1/iphone/location-records":
			err = d.LocationRecords(r.Context(), p, request.Payload)
			result = map[string]bool{"accepted": err == nil}
		case "/v1/iphone/live-disable":
			err = d.Authority.SetIPhoneLiveToken(p.ID, "", "")
			result = map[string]bool{"disabled": err == nil}
		case "/v1/iphone/live-token":
			var token struct {
				Task  string `json:"task_id"`
				Token string `json:"token"`
			}
			if contracts.DecodeInput(request.Payload, &token) != nil {
				err = contracts.ErrInvalid
			} else {
				err = d.Authority.SetIPhoneLiveToken(p.ID, token.Task, token.Token)
				d.Wake()
			}
			result = map[string]bool{"registered": err == nil}
		case "/v1/iphone/push-token":
			var token struct {
				Token string `json:"token"`
			}
			if contracts.DecodeInput(request.Payload, &token) != nil {
				err = contracts.ErrInvalid
			} else {
				err = d.Authority.SetIPhonePushToken(p.ID, token.Token)
				result = map[string]bool{"registered": err == nil}
				d.Wake()
			}
		case "/v1/iphone/health":
			var batch contracts.HealthBatch
			if contracts.DecodeInput(request.Payload, &batch) != nil {
				err = contracts.ErrInvalid
			} else {
				err = d.State.IngestHealth(r.Context(), p.ID, batch)
			}
			result = map[string]bool{"accepted": err == nil}
		case "/v1/iphone/pending":
			result, err = d.packets(r.Context(), p, true)
		case "/v1/iphone/approval":
			var decision struct {
				ID      string `json:"command_id"`
				Hash    string `json:"request_hash"`
				Approve *bool  `json:"approve"`
			}
			if contracts.DecodeInput(request.Payload, &decision) != nil || decision.Approve == nil || d.Decide == nil {
				err = contracts.ErrInvalid
			} else if strings.HasPrefix(decision.ID, "connector:") && d.ConnectorDecide != nil {
				err = d.ConnectorDecide(decision.ID, decision.Hash, *decision.Approve)
			} else {
				_, err = d.Decide(r.Context(), decision.ID, decision.Hash, *decision.Approve)
			}
			result = map[string]bool{"accepted": err == nil}
		case "/v1/iphone/result":
			_, err = d.Complete(r.Context(), p, request)
			result = map[string]bool{"accepted": err == nil}
		default:
			http.NotFound(w, r)
			return
		}
		if err != nil {
			http.Error(w, "rejected", 400)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(result)
	})
}

func (d *Delivery) Run(ctx context.Context) {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()
	d.Wake()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		case <-d.wake:
		}
		d.Sweep(ctx)
		phones, err := d.Authority.IPhones()
		if err != nil {
			continue
		}
		for _, phone := range phones {
			p, err := d.Authority.Principal(phone.ID)
			if err != nil {
				continue
			}
			packets, err := d.packets(ctx, p, d.Push != nil && p.IPhonePushToken != "")
			if err != nil {
				continue
			}
			if d.Push == nil || p.IPhonePushToken == "" {
				continue
			}
			for _, packet := range packets {
				bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
				d.Push(bounded, p.IPhonePushToken, packet, time.Now().Add(time.Minute))
				cancel()
			}
		}
	}
}

func (d *Delivery) Sweep(ctx context.Context) {
	d.mu.Lock()
	defer d.mu.Unlock()
	rows, err := d.State.Commands(ctx)
	if err != nil {
		return
	}
	for _, r := range rows {
		if r.Source != contracts.IPhone || (r.State != contracts.CommandAdmitted && r.State != contracts.CommandPrepared && r.State != contracts.CommandDispatched) {
			continue
		}
		phone, err := d.Authority.Principal(targetOf(r))
		expired := !time.Now().Before(r.ExpiresAt)
		if !expired && err == nil && phone.Kind == "iphone" {
			continue
		}
		to, disposition := contracts.CommandRejected, contracts.DispositionRejectedBeforeDispatch
		if expired {
			to, disposition = contracts.CommandExpired, contracts.DispositionExpired
		}
		if r.State == contracts.CommandDispatched {
			to, disposition = contracts.CommandIndeterminate, contracts.DispositionIndeterminate
		}
		d.transition(ctx, r, to, disposition, nil)
	}
}
