package scenarios

import (
	"context"
	"encoding/json"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runMailReplica(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	check := func(name string, ok bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		return ok
	}
	w, err := world.New(ctx, "replica", 1, 0, world.BlobOptions{Scope: "mail:read,system:read"})
	if !check("ubuntu_replica_started", err == nil) {
		return e
	}
	defer w.Close()
	tag := contracts.NewID("mailtag")
	data, _ := json.Marshal(map[string]any{"account_id": "synthetic@example.com", "message_id": "<fixture@example.com>", "read": false, "to": nil, "cc": nil, "sent_at": nil, "flagged": nil, "flag_color": nil, "size": nil, "snippet": nil, "subject": tag, "from": "sender@example.com", "received_at": "2026-10-28T00:00:00Z", "mailbox_ids": []string{"mailbox/1"}, "body_state": nil, "has_attachments": nil})
	observations := []contracts.Observation{{Kind: "upsert", Collection: "mail.message", NativeKey: tag, Data: data, ObservedAt: time.Now().UTC()}}
	err = w.ControlAction("/dataset", map[string]any{"records": observations})
	if !check("metadata_only_fixture", err == nil) {
		return e
	}
	err = w.Await(20*time.Second, func() bool {
		results := []contracts.ReadResult{{}, {}}
		for i, endpoint := range []world.Endpoint{w.Gateway, w.Replicas[0]} {
			_, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Text: tag, Limit: 100}, &results[i])
			if err != nil || len(results[i].Records) != 1 || results[i].Records[0].Collection != "mail.message" {
				return false
			}
		}
		return reflect.DeepEqual(results[0].Records, results[1].Records)
	})
	e.Passed = check("ubuntu_records_and_subject_fts_equal", err == nil)
	return e
}
