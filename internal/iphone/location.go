package iphone

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type LocationRecord struct {
	ID        string          `json:"id"`
	Kind      string          `json:"kind"`
	Name      *string         `json:"name,omitempty"`
	Fix       json.RawMessage `json:"fix,omitempty"`
	Fence     json.RawMessage `json:"fence,omitempty"`
	Timestamp float64         `json:"timestamp"`
}

func (d *Delivery) LocationRecords(ctx context.Context, p trust.Principal, raw []byte) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	if p.Kind != "iphone" {
		return contracts.ErrUnauthorized
	}
	var records []LocationRecord
	if json.Unmarshal(raw, &records) != nil || len(records) == 0 || len(records) > 25 {
		return contracts.ErrInvalid
	}
	checkpoints, err := d.State.Checkpoints(ctx)
	if err != nil {
		return err
	}
	checkpoint := checkpoints[contracts.IPhone]
	capture := contracts.Capture{Source: contracts.IPhone, Generation: "iphone-location/1", Checkpoint: contracts.NewID("cp")}
	if checkpoint.Generation == capture.Generation {
		capture.Predecessor = checkpoint.Checkpoint
	}
	for _, record := range records {
		if !contracts.ValidCommandID(record.ID) || (record.Kind != "current" && record.Kind != "arrive" && record.Kind != "leave" && record.Kind != "significant" && record.Kind != "geofence" && record.Kind != "geofence_removed") {
			return contracts.ErrInvalid
		}
		at := time.UnixMilli(int64((record.Timestamp + 978307200) * 1000)).UTC()
		data, err := json.Marshal(struct {
			LocationRecord
			DeviceID string `json:"device_id"`
		}{record, p.ID})
		if err != nil {
			return err
		}
		capture.Observations = append(capture.Observations, contracts.Observation{Kind: "upsert", Collection: "iphone.location", NativeKey: p.ID + ":" + record.ID, Data: data, ObservedAt: at})
	}
	_, err = d.State.CommitCapture(ctx, capture)
	return err
}
