package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runFileCommands(ctx context.Context, s Scenario) Evidence {
	rest := runFileCommandsProtocol(ctx, s, "rest")
	if !rest.Passed {
		return rest
	}
	mcp := runFileCommandsProtocol(ctx, s, "mcp")
	for _, a := range mcp.Assertions {
		a.Name = "mcp_" + a.Name
		rest.Assertions = append(rest.Assertions, a)
	}
	rest.Passed = rest.Passed && mcp.Passed
	return rest
}

func runFileCommandsProtocol(ctx context.Context, s Scenario, protocol string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: "sim", At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
		return ok
	}
	ctx = world.WithOptions(ctx, world.Options{Scope: "files:read,files:write,reminders:read,system:read"})
	w, err := world.New(ctx, "sim", 0, 0)
	if !add("gateway_start", err == nil) {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
		}
		return e
	}
	defer w.Close()
	w.Control.Timeout = 30 * time.Second
	data, _ := json.Marshal(map[string]any{"root": "/Bridge Test", "path": "folder", "name": "folder", "kind": "directory", "size": 0, "created": nil, "modified": nil, "tags": []string{}, "downloaded": true, "icloud_status": "local"})
	err = w.ControlAction("/dataset", map[string]any{"records": []contracts.Observation{{Kind: "upsert", Collection: "files.item", NativeKey: "/Bridge Test/folder", Data: data, ObservedAt: time.Now().UTC()}}})
	if !add("directory_fixture", err == nil) {
		fmt.Fprintln(os.Stderr, err)
		return e
	}
	client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
	err = w.Await(5*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &out) == nil
	})
	if !add("helper_ready", err == nil) {
		return e
	}

	run := func(op string, args contracts.FileArgs) bool {
		args.Root = "/Bridge Test"
		raw, _ := json.Marshal(args)
		var out contracts.CommandRecord
		err := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &out)
		if err != nil || out.State != contracts.CommandObserved {
			fmt.Fprintln(os.Stderr, op, err, out.State, out.Error)
		}
		return add(op, err == nil && out.State == contracts.CommandObserved)
	}
	sum := sha256.Sum256([]byte("first"))
	if !run("files.create", contracts.FileArgs{Name: "a.txt", Content: base64.StdEncoding.EncodeToString([]byte("first")), Digest: hex.EncodeToString(sum[:])}) {
		return e
	}
	sum = sha256.Sum256([]byte("second"))
	if !run("files.write", contracts.FileArgs{Name: "a.txt", Content: base64.StdEncoding.EncodeToString([]byte("second")), Digest: hex.EncodeToString(sum[:])}) {
		return e
	}
	var search contracts.QueryResult
	err = client.Call(ctx, "files.search", contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"query":"a.txt","field":"name"}`)}, &search)
	var matches struct {
		Items []json.RawMessage `json:"items"`
	}
	if !add("created_item_search", err == nil && json.Unmarshal(search.ResultJSON, &matches) == nil && len(matches.Items) == 1) {
		return e
	}
	var read contracts.QueryResult
	err = client.Call(ctx, "files.read", contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"root":"/Bridge Test","path":"a.txt"}`)}, &read)
	if !add("content_digest_readback", err == nil && len(read.Blobs) == 1 && read.Blobs[0].Digest == hex.EncodeToString(sum[:])) {
		return e
	}
	if !run("files.rename", contracts.FileArgs{Path: "a.txt", Name: "b.txt"}) {
		return e
	}
	if !run("files.move", contracts.FileArgs{Path: "b.txt", Parent: "folder", Name: "c.txt"}) {
		return e
	}
	if !run("files.set_tags", contracts.FileArgs{Path: "folder/c.txt", Tags: []string{"Bridge Test"}}) {
		return e
	}
	if !run("files.evict", contracts.FileArgs{Path: "folder/c.txt"}) {
		return e
	}
	if !run("files.download", contracts.FileArgs{Path: "folder/c.txt"}) {
		return e
	}
	records, err := w.Records(w.Gateway)
	found := false
	for _, r := range records {
		if r.Collection == "files.item" {
			var data struct {
				Path       string   `json:"path"`
				Tags       []string `json:"tags"`
				Downloaded bool     `json:"downloaded"`
			}
			json.Unmarshal(r.Data, &data)
			if data.Path == "folder/c.txt" && len(data.Tags) == 1 && data.Tags[0] == "Bridge Test" && data.Downloaded {
				found = true
			}
		}
	}
	if !add("captured_metadata_readback", err == nil && found) {
		return e
	}
	for _, p := range []string{"../escape", "/outside"} {
		raw, _ := json.Marshal(contracts.FileArgs{Root: "/Bridge Test", Path: p})
		var out contracts.CommandRecord
		err = client.Call(ctx, "files.trash", contracts.Command{ID: contracts.NewCommandID(), Args: raw}, &out)
		add("escape_rejected", err != nil || out.State == contracts.CommandRejected)
	}
	run("files.trash", contracts.FileArgs{Path: "folder/c.txt"})
	return e
}
