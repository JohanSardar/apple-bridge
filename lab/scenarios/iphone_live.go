package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
)

func runIPhoneLive(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	if level == "real" {
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "phone_live_widgets", Status: "limited", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	check := func(name string, passed bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		return passed
	}
	dir, err := os.MkdirTemp("", "ab-test-live-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	gateway, phone, token, err := iphoneBinaryFixture(ctx, dir)
	if !check("pair", err == nil) {
		e.Assertions[len(e.Assertions)-1].Detail = fmt.Sprint(err)
		return e
	}
	defer gateway.Close()
	register := func(task, token string) bool {
		raw, _ := json.Marshal(map[string]string{"task_id": task, "token": token})
		response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/live-token", raw)
		if err != nil {
			return false
		}
		defer response.Body.Close()
		return response.StatusCode == 200
	}
	if !check("signed_start_and_task_token_registration", register("_start", strings.Repeat("a", 64)) && register("ab-test", strings.Repeat("b", 64))) {
		return e
	}
	sink, err := phonefixture.LiveSink(gateway)
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "relay_sink", Detail: err.Error()})
		return e
	}
	defer sink.Close()
	for index, action := range []string{"start", "update", "end"} {
		args, _ := json.Marshal(contracts.IPhoneLiveArgs{DeviceID: phone.ID, TaskID: "ab-test", Title: "[ab-test] task", Status: action, Progress: 0.5})
		command := contracts.Command{ID: []string{"cmd_live_z", "cmd_live_m", "cmd_live_a"}[index], Op: "iphone.live." + action, Args: args}
		var admitted contracts.CommandRecord
		err := gateway.Invoke(command.Op, command, token, &admitted)
		if !check(action+"_admitted", err == nil && admitted.State == contracts.CommandAdmitted) {
			return e
		}
	}
	response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	if !check("three_apns_lifecycle_events_without_duplicate_start", sink.Wait("start,update,end")) {
		log, _ := os.ReadFile(filepath.Join(dir, "gateway.log"))
		e.Assertions[len(e.Assertions)-1].Detail = sink.Detail() + " " + string(log)
		return e
	}
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/live-disable", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	raw, err := os.ReadFile(filepath.Join(gateway.Home, "credentials.json"))
	var credentials struct {
		Principals []struct {
			ID     string            `json:"id"`
			Tokens map[string]string `json:"iphone_live_tokens"`
		} `json:"principals"`
	}
	valid := err == nil && json.Unmarshal(raw, &credentials) == nil
	found := false
	for _, principal := range credentials.Principals {
		if principal.ID == phone.ID {
			found = true
			valid = valid && len(principal.Tokens) == 0
		}
	}
	if !check("disable_revokes_live_tokens", valid && found) {
		return e
	}

	e.Passed = true
	return e
}
