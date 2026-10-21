package main

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket/wsjson"
	"os"
	"time"
)

func revocation(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	decoder := json.NewDecoder(os.Stdin)
	var in input
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	authority, err := trust.Open(in.Home)
	if err != nil {
		return err
	}
	enrollment, err := authority.Enrollment()
	if err != nil || enrollment == nil {
		return fmt.Errorf("enrollment missing")
	}
	conn, err := publicDial(ctx, in, *enrollment)
	if err != nil {
		return err
	}
	defer conn.CloseNow()
	principal, err := authority.RelayClient(ctx, conn, enrollment.Fingerprint)
	if err != nil || principal.ID != enrollment.ReplicaID {
		return fmt.Errorf("revocation connection unauthenticated")
	}
	conn.SetReadLimit(contracts.MaxFrame * 2)
	var local contracts.ReadResult
	if err = request(ctx, in, "records.query", contracts.ReadQuery{Limit: 1}, &local); err != nil {
		return err
	}
	if err = wsjson.Write(ctx, conn, contracts.FeedFrame{T: "Hello", Protocol: contracts.FeedProtocol, ReplicaID: enrollment.ReplicaID, Cursor: &local.Cursor, SchemaOffer: []int{1}, Credits: contracts.ByteCredit}); err != nil {
		return err
	}
	var welcome contracts.FeedFrame
	if err = wsjson.Read(ctx, conn, &welcome); err != nil || welcome.T != "Welcome" {
		return fmt.Errorf("revocation feed not welcomed")
	}
	closed := make(chan int64, 1)
	go func() {
		for {
			var frame contracts.FeedFrame
			if wsjson.Read(ctx, conn, &frame) != nil {
				if ctx.Err() == nil {
					closed <- time.Now().UnixNano()
				} else {
					closed <- 0
				}
				return
			}
			if frame.Batch != nil {
				cursor := contracts.Cursor{Epoch: frame.Batch.Epoch, SubscriptionRevision: frame.Batch.SubscriptionRevision, Sequence: frame.Batch.Through}
				wsjson.Write(ctx, conn, contracts.FeedFrame{T: "Ack", Cursor: &cursor, Credits: contracts.ByteCredit})
			}
		}
	}()
	if err = json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true}); err != nil {
		return err
	}
	var signal any
	if err = decoder.Decode(&signal); err != nil {
		return err
	}
	var clock contracts.ClockSample
	raw, err := os.ReadFile(in.Home + "/gateway-clock.json")
	if err != nil {
		return err
	}
	if err = json.Unmarshal(raw, &clock); err != nil {
		return err
	}
	offset, uncertainty, err := clock.Estimate()
	if err != nil {
		return err
	}
	select {
	case at := <-closed:
		if at == 0 {
			return fmt.Errorf("revocation close unverified")
		}
		return json.NewEncoder(os.Stdout).Encode(map[string]any{"closed_gateway_wall_ns": at + int64(offset), "clock_uncertainty_ms": float64(uncertainty) / float64(time.Millisecond)})
	case <-ctx.Done():
		return ctx.Err()
	}
}
