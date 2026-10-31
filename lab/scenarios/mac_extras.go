package scenarios

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"time"
)

func runMacExtras(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	if level != "sim" || s.ID == "IWORK-REAL" {
		return runMacExtrasReal(ctx, s, level)
	}
	ctx = world.WithOptions(ctx, world.Options{Scope: "mac:read,mac:write,system:read"})
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "mac:read,mac:write,reminders:read,system:read"})
	add("world_started", err == nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, "mac extras world startup:", err)
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "mac:read,mac:write,system:read")
	add("scope_granted", err == nil)
	client := scripted.Client{Endpoint: endpoint}
	err = w.Await(20*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
	})
	add("helper_ready", err == nil)
	if err != nil {
		return
	}
	err = w.ControlAction("/mac", map[string]any{"collection": "mac.bluetooth_device", "key": "bridge-test-headphones", "data": map[string]any{"name": "Bridge Test Headphones", "connected": true, "battery_percent": 75}})
	add("bluetooth_capture_injected", err == nil)
	err = w.Await(time.Second, func() bool {
		var out contracts.ReadResult
		return client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "mac.bluetooth_device", Limit: 10}, &out) == nil && len(out.Records) == 1 && len(contracts.CheckShape("mac.bluetooth_device", out.Records[0].Data)) == 0
	})
	add("bluetooth_capture_visible", err == nil)
	for _, test := range []struct {
		op, args, field string
		expected        any
	}{
		{"mac.bluetooth_devices", `{}`, "devices", nil},
		{"mac.time_machine_status", `{}`, "running", false},
		{"mac.recognize_song", `{"audio_blob":"YnJpZGdlLWxhYi1hdWRpbw=="}`, "title", "Bridge Test Song"},
	} {
		var out contracts.QueryResult
		err = client.Call(ctx, test.op, contracts.Query{Args: json.RawMessage(test.args)}, &out)
		var result map[string]any
		decode := json.Unmarshal(out.ResultJSON, &result)
		ok := err == nil && decode == nil && result[test.field] != nil
		if test.expected != nil {
			ok = ok && result[test.field] == test.expected
		}
		if test.expected == nil {
			items, valid := result[test.field].([]any)
			ok = ok && valid && len(items) == 1
		}
		add(test.op+"_fixture", ok)
	}
	for _, test := range []struct {
		op, args, field string
		expected        any
	}{
		{"mac.iwork_export", `{"app":"pages","format":"text","document_blob":"YnJpZGdlLWxhYg=="}`, "data_base64", "YnJpZGdlLWxhYg=="},
	} {
		var out contracts.CommandRecord
		err = client.Call(ctx, test.op, contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(test.args), WaitMS: 15000}, &out)
		var evidence struct {
			Readback map[string]any `json:"readback"`
		}
		decode := json.Unmarshal(out.Evidence, &evidence)
		add(test.op+"_readback", err == nil && out.State == contracts.CommandObserved && contracts.MacCompletionEvidence(out.Evidence, out.RequestHash) && decode == nil && evidence.Readback[test.field] == test.expected)
	}
	for _, app := range []string{"pages", "numbers", "keynote"} {
		for _, format := range []string{"pdf", "text"} {
			raw, _ := json.Marshal(map[string]any{"app": app, "format": format, "document_blob": "YnJpZGdlLWxhYg=="})
			var out contracts.CommandRecord
			err = client.Call(ctx, "mac.iwork_export", contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &out)
			var evidence struct {
				Readback struct {
					Data string `json:"data_base64"`
				} `json:"readback"`
			}
			decode := json.Unmarshal(out.Evidence, &evidence)
			data, blobErr := base64.StdEncoding.DecodeString(evidence.Readback.Data)
			valid := bytes.Equal(data, []byte("bridge-lab"))
			if format == "pdf" {
				valid = bytes.HasPrefix(data, []byte("%PDF-1.4")) && bytes.Contains(data, []byte("bridge-lab")) && bytes.HasSuffix(data, []byte("%%EOF\n"))
			}
			add(app+"_"+format+"_export", err == nil && decode == nil && blobErr == nil && out.State == contracts.CommandObserved && valid)
		}
	}
	for _, op := range []string{"mac.iwork_export"} {
		var out contracts.CommandRecord
		err = client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{}`)}, &out)
		add(op+"_invalid_rejected", err != nil)
	}
	var song contracts.QueryResult
	err = client.Call(ctx, "mac.recognize_song", contracts.Query{Args: json.RawMessage(`{"audio_blob":""}`)}, &song)
	add("invalid_audio_rejected", err != nil)
	return
}
