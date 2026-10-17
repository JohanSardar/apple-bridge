package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func aliasKey(source contracts.Source, collection, native string) string {
	return string(source) + "/" + collection + "/" + native
}
func gatewayAliases(w *world.World) (map[string]string, error) {
	result := map[string]string{}
	after := ""
	for {
		var page contracts.ReadResult
		_, err := world.Request(context.Background(), w.Gateway, "/v1/op/records.query", contracts.ReadQuery{IncludeAliases: true, IncludeDeleted: true, After: after, Limit: 1000}, &page)
		if err != nil {
			return nil, err
		}
		for _, a := range page.Aliases {
			key := aliasKey(a.Source, a.Collection, a.NativeKey)
			if id, ok := result[key]; ok && id != a.RecordID {
				return nil, fmt.Errorf("ambiguous alias")
			}
			result[key] = a.RecordID
		}
		if page.Next == "" {
			return result, nil
		}
		after = page.Next
	}
}
func resolveTruth(value any, source contracts.Source, aliases map[string]string) (any, error) {
	switch v := value.(type) {
	case map[string]any:
		if ref, ok := v["$ref"].(map[string]any); ok {
			collection, _ := ref["collection"].(string)
			native, _ := ref["native_key"].(string)
			id := aliases[aliasKey(source, collection, native)]
			if id == "" {
				return nil, fmt.Errorf("Truth reference has no bridge alias")
			}
			return id, nil
		}
		for key, item := range v {
			resolved, err := resolveTruth(item, source, aliases)
			if err != nil {
				return nil, err
			}
			v[key] = resolved
		}
	case []any:
		for i, item := range v {
			resolved, err := resolveTruth(item, source, aliases)
			if err != nil {
				return nil, err
			}
			v[i] = resolved
		}
	}
	return value, nil
}
func truthRecords(observations []contracts.Observation, aliases map[string]string) ([]contracts.Record, error) {
	records := []contracts.Record{}
	for _, o := range observations {
		if o.Kind == "absent" {
			continue
		}
		source := contracts.CollectionSource(o.Collection)
		id := aliases[aliasKey(source, o.Collection, o.NativeKey)]
		if id == "" {
			return nil, fmt.Errorf("Truth record has no bridge alias")
		}
		var data any
		if err := decodeData(o.Data, &data); err != nil {
			return nil, err
		}
		resolved, err := resolveTruth(data, source, aliases)
		if err != nil {
			return nil, err
		}
		raw, err := json.Marshal(resolved)
		if err != nil {
			return nil, err
		}
		records = append(records, contracts.Record{ID: id, Collection: o.Collection, Source: source, Data: raw})
	}
	return records, nil
}
func recordMultiset(records []contracts.Record, full bool) map[string]int {
	result := map[string]int{}
	for _, r := range records {
		if !full && r.Collection == "system.source" {
			continue
		}
		var data any
		decodeData(r.Data, &data)
		r.Data, _ = json.Marshal(data)
		var value any = r
		if !full {
			value = struct {
				ID         string
				Collection string
				Source     contracts.Source
				Deleted    bool
				Data       json.RawMessage
			}{r.ID, r.Collection, r.Source, r.Deleted, r.Data}
		}
		raw, _ := json.Marshal(value)
		result[string(raw)]++
	}
	return result
}
func readConsistent(w *world.World, e world.Endpoint) (contracts.ReadResult, error) {
	result := contracts.ReadResult{}
	after := ""
	for {
		var page contracts.ReadResult
		_, err := world.Request(context.Background(), e, "/v1/op/records.query", contracts.ReadQuery{IncludeDeleted: true, After: after, Limit: 1000}, &page)
		if err != nil {
			return result, err
		}
		if after != "" && page.Cursor != result.Cursor {
			return result, fmt.Errorf("cursor changed during pagination")
		}
		result.Cursor = page.Cursor
		result.Records = append(result.Records, page.Records...)
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	head, err := w.Cursor(e)
	if err != nil {
		return result, err
	}
	if head != result.Cursor {
		return result, fmt.Errorf("cursor changed after read")
	}
	return result, nil
}
func replicaIntegrity(w *world.World) error {
	gateway, err := readConsistent(w, w.Gateway)
	if err != nil {
		return err
	}
	for i, e := range w.Replicas {
		replica, err := readConsistent(w, e)
		if err != nil {
			return err
		}
		if replica.Cursor != gateway.Cursor {
			return fmt.Errorf("replica%d cursor differs from gateway", i)
		}
		if !reflect.DeepEqual(recordMultiset(replica.Records, true), recordMultiset(gateway.Records, true)) {
			return fmt.Errorf("replica%d records or versions differ at cursor", i)
		}
	}
	head, err := w.Cursor(w.Gateway)
	if err != nil {
		return err
	}
	if head != gateway.Cursor {
		return fmt.Errorf("gateway moved during integrity check")
	}
	return nil
}
func recordIntegrity(e *Evidence, w *world.World, apply *Assertion) {
	assertion := Assertion{Name: "replica_records_versions_and_cursor_consistent", Status: "unverified", Detail: "no replica checked"}
	if len(w.Replicas) > 0 {
		var check error
		err := w.Await(5*time.Second, func() bool { check = replicaIntegrity(w); return check == nil })
		assertion.Passed = err == nil
		assertion.Status = "fail"
		if assertion.Passed {
			assertion.Status = "pass"
			assertion.Detail = "public records query, including versions and tombstones, matched at stable full cursors"
		} else if check != nil {
			assertion.Detail = check.Error()
		}
	}
	e.Assertions = append(e.Assertions, assertion)
	if apply == nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "replica_application_exactly_once", Status: "unverified", Detail: "kill-during-apply check did not run"})
	} else {
		e.Assertions = append(e.Assertions, *apply)
	}
}
