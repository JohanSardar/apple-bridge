package state

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) IngestHealth(ctx context.Context, device string, b contracts.HealthBatch) error {
	if b.Stream == "" || len(b.Stream) > 160 || b.Sequence == 0 || b.Sequence > math.MaxInt64 || len(b.Samples)+len(b.Deleted) > 100 {
		return contracts.ErrInvalid
	}
	for _, sample := range b.Samples {
		if sample.ID == "" || sample.Type != b.Stream || sample.Start.IsZero() || sample.End.Before(sample.Start) || sample.Value != nil && (math.IsNaN(*sample.Value) || math.IsInf(*sample.Value, 0)) {
			return contracts.ErrInvalid
		}
	}
	for _, id := range b.Deleted {
		if id == "" {
			return contracts.ErrInvalid
		}
	}
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS health_streams(device TEXT,stream TEXT,sequence INTEGER,digest TEXT,PRIMARY KEY(device,stream))`); err != nil {
		return err
	}
	raw, err := json.Marshal(b)
	if err != nil {
		return err
	}
	digest := fmt.Sprintf("%x", sha256.Sum256(raw))
	var seq uint64
	var prior string
	err = tx.QueryRowContext(ctx, "SELECT sequence,digest FROM health_streams WHERE device=? AND stream=?", device, b.Stream).Scan(&seq, &prior)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	if seq == b.Sequence && prior == digest {
		return nil
	}
	if b.Sequence != seq+1 {
		return contracts.ErrConflict
	}
	c, err := cursorAt(ctx, tx)
	if err != nil {
		return err
	}
	writer := newRecordWriter(tx)
	defer writer.close()
	write := func(id string, sample *contracts.HealthSample) error {
		key := sha256.Sum256([]byte(device + "\x00" + b.Stream + "\x00" + id))
		rid := "hsm_" + hex.EncodeToString(key[:])
		old, e := writer.recordAt(ctx, "health.sample", rid)
		if e != nil && !errors.Is(e, sql.ErrNoRows) {
			return e
		}
		data := old.Data
		if sample != nil {
			data, e = json.Marshal(struct {
				contracts.HealthSample
				DeviceID string `json:"device_id"`
			}{*sample, device})
			if e != nil {
				return e
			}
		}
		if sample == nil && len(data) == 0 {
			data = json.RawMessage(`{}`)
		}
		r := contracts.Record{Collection: "health.sample", ID: rid, Version: old.Version + 1, Source: "health", ObservedAt: time.Now().UTC(), Deleted: sample == nil, Data: data, Coverage: contracts.Coverage{Fields: "complete", Missing: []string{}}}
		c.Sequence++
		op := "upsert"
		if r.Deleted {
			op = "delete"
		}
		if e = writer.writeRecord(ctx, r, c.Sequence); e != nil {
			return e
		}
		return writer.appendChange(ctx, c, r, op)
	}
	for _, sample := range b.Samples {
		if err = write(sample.ID, &sample); err != nil {
			return err
		}
	}
	for _, id := range b.Deleted {
		if err = write(id, nil); err != nil {
			return err
		}
	}
	if err = setCursor(ctx, tx, c); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO health_streams(device,stream,sequence,digest) VALUES(?,?,?,?) ON CONFLICT(device,stream) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest", device, b.Stream, b.Sequence, digest); err != nil {
		return err
	}
	return s.commitProtected(ctx, tx)
}

func (s *Store) HealthSummary(ctx context.Context, v trust.Visibility, q contracts.HealthSummaryQuery) (contracts.HealthSummary, error) {
	out := contracts.HealthSummary{Metrics: map[string]contracts.HealthMetric{}}
	if !v["health"] {
		return out, contracts.ErrUnauthorized
	}
	if q.Period != "day" && q.Period != "week" {
		return out, contracts.ErrInvalid
	}
	zone := q.TimeZone
	if zone == "" {
		zone = "UTC"
	}
	loc, err := time.LoadLocation(zone)
	if err != nil {
		return out, contracts.ErrInvalid
	}
	out.Start, err = time.ParseInLocation("2006-01-02", q.Date, loc)
	if err != nil {
		return out, contracts.ErrInvalid
	}
	days := 1
	if q.Period == "week" {
		days = 7
		out.Start = out.Start.AddDate(0, 0, -(int(out.Start.Weekday())+6)%7)
	}
	out.End = out.Start.AddDate(0, 0, days)
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer tx.Rollback()
	out.Cursor, err = cursorAt(ctx, tx)
	if err != nil {
		return out, err
	}
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE collection='health.sample' AND deleted=0")
	if err != nil {
		return out, err
	}
	defer rows.Close()
	for rows.Next() {
		var raw []byte
		var r contracts.Record
		var f struct {
			contracts.HealthSample
			DeviceID string `json:"device_id"`
		}
		if err = rows.Scan(&raw); err != nil {
			return out, err
		}
		if err = json.Unmarshal(raw, &r); err != nil {
			return out, err
		}
		if err = json.Unmarshal(r.Data, &f); err != nil {
			return out, err
		}
		if q.DeviceID != "" && q.DeviceID != f.DeviceID {
			continue
		}
		if !f.Start.Before(out.End) || f.End.Before(out.Start) || f.End.Equal(out.Start) && f.Start.Before(out.Start) {
			continue
		}
		key := f.Type + "/" + f.Unit
		if f.Category != nil {
			key += fmt.Sprintf("/%d", *f.Category)
		}
		key = strings.TrimSuffix(key, "/")
		m := out.Metrics[key]
		m.Count++
		if f.Value != nil {
			if f.Start.Before(out.Start) {
				continue
			}
			value := *f.Value
			m.Sum += value
			if m.Count == 1 || value < m.Min {
				m.Min = value
			}
			if m.Count == 1 || value > m.Max {
				m.Max = value
			}
			m.Average = m.Sum / float64(m.Count)
		}
		start, end := f.Start, f.End
		if start.Before(out.Start) {
			start = out.Start
		}
		if end.After(out.End) {
			end = out.End
		}
		m.Seconds += end.Sub(start).Seconds()
		out.Metrics[key] = m
	}
	return out, rows.Err()
}
