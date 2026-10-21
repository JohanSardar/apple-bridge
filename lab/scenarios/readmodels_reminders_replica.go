package scenarios

import (
	"context"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelReplica(ctx context.Context, w *world.World, add func(string, bool)) {
	err := w.AddReplicaScope("reminders:read,system:read")
	add("replica_added_after_reminders_for_snapshot", err == nil)
	if err != nil {
		return
	}
	for _, operation := range []string{"reminders.lists"} {
		query := map[string]any{"time_zone": "America/New_York"}
		var expected map[string]any
		err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, operation, query, &expected)
		add(operation+"_gateway_before_replica", err == nil)
		if err != nil {
			continue
		}
		delete(expected, "cursor")
		for index, endpoint := range w.Replicas {
			err = w.Await(10*time.Second, func() bool {
				var actual map[string]any
				err := (scripted.Client{Endpoint: endpoint}).Call(ctx, operation, query, &actual)
				delete(actual, "cursor")
				return err == nil && reflect.DeepEqual(expected, actual)
			})
			name := "live_apply"
			if index == len(w.Replicas)-1 {
				name = "snapshot_rebuild"
			}
			add(operation+"_"+name+"_matches_gateway", err == nil)
		}
	}
}
