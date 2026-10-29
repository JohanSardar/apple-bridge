package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"reflect"
	"time"
)

func safariGateway(ctx context.Context, e Evidence, observations []contracts.Observation) Evidence {
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	for _, row := range observations {
		add("registered_shape_"+row.Collection, contracts.HasShape(row.Collection) && len(contracts.CheckShape(row.Collection, row.Data)) == 0)
	}
	grants, known := contracts.GrantsForSource(contracts.Safari)
	add("read_and_write_grants_registered", known && len(grants.Grants) == 2 && grants.Grants[0] == "safari:read" && grants.Grants[1] == "safari:write")
	w, err := world.New(ctx, "sim", 0, 0)
	add("gateway_world", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	token, err := w.ScopedToken(w.Gateway, "safari:read,system:read")
	add("safari_read_scope", err == nil)
	if err != nil {
		return e
	}
	w.Gateway.Token = token
	add("safari_replica_scope", w.AddReplicaScope("safari:read,system:read") == nil)
	if !e.Passed {
		return e
	}
	add("simulated_native_capture", w.ControlAction("/safari", observations) == nil)
	matches := func(endpoint world.Endpoint) bool {
		rows, err := w.Records(endpoint)
		if err != nil {
			return false
		}
		found := 0
		matched := make([]bool, len(observations))
		for _, row := range rows {
			if row.Source != contracts.Safari || contracts.CollectionSource(row.Collection) != contracts.Safari || row.Deleted {
				continue
			}
			match := false
			for i, observation := range observations {
				if matched[i] || row.Collection != observation.Collection {
					continue
				}
				var actual, want any
				if json.Unmarshal(row.Data, &actual) == nil && json.Unmarshal(observation.Data, &want) == nil && reflect.DeepEqual(actual, want) {
					matched[i] = true
					match = true
					break
				}
			}
			if !match {
				return false
			}
			found++
		}
		return found == len(observations)
	}
	add("gateway_fields_match_native_fixture", w.Await(15*time.Second, func() bool { return matches(w.Gateway) }) == nil)
	add("replica_fields_match_native_fixture", w.Await(15*time.Second, func() bool { return matches(w.Replicas[0]) }) == nil)
	return e
}
