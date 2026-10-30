package scenarios

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
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
	authority, err := trust.Open(filepath.Join(dir, "trust"))
	if err != nil {
		return e
	}
	store, err := state.Open(filepath.Join(dir, "state"))
	if err != nil {
		return e
	}
	defer store.Close()
	if store.InitCommands(ctx) != nil {
		return e
	}
	delivery := iphone.NewDelivery(store, authority)
	gateway := &operations.LocalGateway{State: store, Authority: authority, Context: ctx, IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
	mux := http.NewServeMux()
	mux.Handle("/v1/iphone/pair", authority.IPhoneHandler())
	mux.Handle("/v1/iphone/", delivery.Handler())
	server := httptest.NewTLSServer(mux)
	defer server.Close()
	code, err := authority.PairIPhoneCode(server.URL, false, time.Minute)
	if err != nil {
		return e
	}
	phone, err := simapple.NewIPhone()
	if err != nil {
		return e
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	if !check("pair", err == nil && status == 200) {
		return e
	}
	register := func(task, token string) bool {
		raw, _ := json.Marshal(map[string]string{"task_id": task, "token": token})
		response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/live-token", raw)
		if err != nil {
			return false
		}
		defer response.Body.Close()
		return response.StatusCode == 200
	}
	if !check("signed_start_and_task_token_registration", register("_start", strings.Repeat("a", 64)) && register("ab-test", strings.Repeat("b", 64))) {
		return e
	}
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	events := []string{}
	pushValid := true
	apnsServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			APS map[string]json.RawMessage `json:"aps"`
		}
		raw, _ := io.ReadAll(r.Body)
		var event string
		if json.Unmarshal(raw, &body) != nil || json.Unmarshal(body.APS["event"], &event) != nil {
			pushValid = false
		}
		expectedToken := strings.Repeat("b", 64)
		if event == "start" {
			expectedToken = strings.Repeat("a", 64)
			pushValid = pushValid && string(body.APS["attributes-type"]) == `"AgentTaskAttributes"`
		}
		pushValid = pushValid && r.URL.Path == "/3/device/"+expectedToken && r.Header.Get("apns-push-type") == "liveactivity" && r.Header.Get("apns-topic") == "com.johansardar.applebridge.ios.push-type.liveactivity" && r.Header.Get("apns-priority") == "10" && len(body.APS["content-state"]) > 0
		events = append(events, event)
		w.WriteHeader(200)
	}))
	defer apnsServer.Close()
	apns := &relay.APNs{Key: key, KeyID: "ab-test", Team: "ab-test", Client: apnsServer.Client(), Endpoint: apnsServer.URL}
	delivery.Push = func(ctx context.Context, token string, packet iphone.Packet, expires time.Time) error {
		raw, _ := json.Marshal(packet)
		return apns.Send(ctx, token, raw, expires)
	}
	token, err := authority.CreateToken([]contracts.Grant{"iphone:write"})
	if err != nil {
		return e
	}
	principal, err := authority.AuthenticateToken(token)
	if err != nil {
		return e
	}
	for index, action := range []string{"start", "update", "end"} {
		args, _ := json.Marshal(contracts.IPhoneLiveArgs{DeviceID: phone.ID, TaskID: "ab-test", Title: "[ab-test] task", Status: action, Progress: 0.5})
		command := contracts.Command{ID: []string{"cmd_live_z", "cmd_live_m", "cmd_live_a"}[index], Op: "iphone.live." + action, Args: args}
		admitted, err := gateway.Submit(ctx, principal, command)
		if !check(action+"_admitted", err == nil && admitted.State == contracts.CommandAdmitted) {
			return e
		}
	}
	response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	if !check("three_apns_lifecycle_events_without_duplicate_start", pushValid && strings.Join(events, ",") == "start,update,end") {
		return e
	}
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/live-disable", nil)
	if err != nil {
		return e
	}
	response.Body.Close()
	p, err := authority.Principal(phone.ID)
	if !check("disable_revokes_live_tokens", err == nil && len(p.IPhoneLiveTokens) == 0) {
		return e
	}
	e.Passed = true
	return e
}
