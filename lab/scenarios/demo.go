package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"os/exec"
	"regexp"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runDemoDataset(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Metrics: map[string]any{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	first := demo.Generate(demo.Options{Seed: 17})
	second := demo.Generate(demo.Options{Seed: 17})
	a, _ := json.Marshal(first)
	b, _ := json.Marshal(second)
	add("same_seed_byte_identical_records_and_blobs", bytes.Equal(a, b) && first.Digest() == second.Digest())
	add("different_seed_changes_dataset", first.Digest() != demo.Generate(demo.Options{Seed: 18}).Digest())
	add("generated_data_synthetic", demoSynthetic(a, first.Blobs))
	e.Metrics["digest"] = first.Digest()
	e.Metrics["counts"] = first.Counts
	started := time.Now()
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "admin"})
	add("sim_world", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	err = demo.Populate(w, first)
	add("demo_dataset_capture", err == nil)
	if err != nil {
		return e
	}
	elapsed := time.Since(started)
	e.Latency["default_startup"] = stats([]float64{float64(elapsed) / float64(time.Millisecond)}, 0)
	add("default_startup_under_10_seconds", elapsed < 10*time.Second)
	rawPairing, _ := json.Marshal(first.Dataset)
	var invalidPairing map[string]any
	json.Unmarshal(rawPairing, &invalidPairing)
	invalidPairing["live_photos"] = map[string]string{"missing-still": "missing-movie"}
	add("demo_rejects_missing_live_photo_companion", w.ControlAction("/dataset", invalidPairing) != nil)
	records, err := w.Records(w.Gateway)
	counts := map[string]int{}
	avatars := 0
	for _, record := range records {
		counts[record.Collection]++
		if record.Collection == "contacts.contact" && rmData(record)["image"] != nil {
			avatars++
		}
	}
	add("captured_60_contacts_40_chats_5000_messages", err == nil && counts["contacts.contact"] == 60 && counts["messages.chat"] == 40 && counts["messages.message"] == 5000)
	add("30_generated_avatars", avatars == 30)
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	var active, archived contracts.ChatsResult
	err = client.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &active)
	archiveErr := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: "archived", Limit: 200}, &archived)
	add("readmodel_37_active_2_archived", err == nil && archiveErr == nil && len(active.Chats) == 37 && len(archived.Chats) == 2)
	if len(active.Chats) > 0 {
		var thread contracts.ThreadResult
		err = client.Call(ctx, "messages.thread", contracts.ThreadQuery{ChatID: active.Chats[0].Chat.ID, Limit: 200}, &thread)
		add("readmodel_thread_visible", err == nil && len(thread.Items) > 0)
	}
	media := demoReadmodelCoverage(ctx, w, &e, add)
	demoMediaCoverage(ctx, w, first, media, e.Metrics["coverage"].(map[string]int), &e, add)
	demoRich(ctx, client, records, e.Metrics["coverage"].(map[string]int), add)
	server, err := demo.Serve(w, "127.0.0.1:0")
	add("demo_loopback_server", err == nil)
	if err == nil {
		defer server.Close()
		demoSessionCheck(ctx, server, w, add)
	}
	observations, err := w.Inspect()
	raw, _ := json.Marshal(observations)
	add("captured_data_synthetic", err == nil && demoSynthetic(raw, nil))
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func demoSynthetic(raw []byte, blobs map[string][]byte) bool {
	email := regexp.MustCompile(`[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}`)
	phone := regexp.MustCompile(`\+[0-9][0-9 -]{6,}[0-9]`)
	check := func(data []byte) bool {
		for _, value := range email.FindAll(data, -1) {
			if !strings.HasSuffix(string(value), "@example.com") {
				return false
			}
		}
		for _, value := range phone.FindAll(data, -1) {
			if !regexp.MustCompile(`^\+1 555 01[0-9]{2}$`).Match(value) {
				return false
			}
		}
		return true
	}
	if !check(raw) {
		return false
	}
	for _, blob := range blobs {
		if bytes.HasPrefix(blob, []byte("BEGIN:VCARD")) || bytes.HasPrefix(blob, []byte("%PDF")) {
			if !check(blob) {
				return false
			}
		}
	}
	return true
}
