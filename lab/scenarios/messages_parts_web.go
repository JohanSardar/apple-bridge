package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func checkWebMessageParts(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		label := "web_gateway"
		if index > 0 {
			label = "web_replica"
		}
		bounded, stop := context.WithTimeout(ctx, 30*time.Second)
		authority, err := trust.Open(endpoint.Home)
		if err != nil {
			add(label+"_authority", false)
			stop()
			return
		}
		token, _, err := authority.CreateDeviceSession("trusted")
		add(label+"_session", err == nil)
		if err != nil {
			stop()
			return
		}
		client := &webClient{ctx: bounded, origin: strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
		files := []contracts.MessageFile{}
		for i := 0; i < 2; i++ {
			status, ready := uploadRequest(client, "POST", "upload", bytes.NewReader([]byte("[ab-test "+evidence.RunID+"] web part "+strconv.Itoa(i))), nil)
			add(label+"_ready_upload_"+strconv.Itoa(i), status == 200 && ready.State == "ready" && ready.ID != "")
			if ready.ID == "" {
				stop()
				return
			}
			files = append(files, contracts.MessageFile{Name: "same.txt", UploadID: ready.ID})
		}
		socket, err := client.socket()
		add(label+"_socket", err == nil)
		if err != nil {
			stop()
			return
		}
		raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] web multipart", "files": files})
		command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw}
		result, err := uploadCommandReply(bounded, socket, command)
		socket.CloseNow()
		add(label+"_observed", err == nil && result.State == contracts.CommandObserved)
		var status messagePartsStatus
		encoded, _ := json.Marshal(result)
		json.Unmarshal(encoded, &status)
		aliases, problem := gatewayAliases(w)
		expected := []messagePartStatus{}
		seen := map[string]bool{}
		for i, part := range status.Evidence.Parts {
			id := aliases[aliasKey(contracts.Messages, "messages.message", part.NativeKey)]
			add(label+"_exact_unique_part", problem == nil && id != "" && !seen[id] && part.Index == i)
			seen[id] = true
			disposition := part.Disposition
			if disposition == "locally_observed" {
				disposition = "observed"
			}
			entry := messagePartStatus{Index: i, Kind: part.Kind, RecordID: id, Disposition: disposition}
			if i > 0 && i <= len(files) {
				entry.UploadID = files[i-1].UploadID
			}
			expected = append(expected, entry)
		}
		add(label+"_cmd_status_parts", len(expected) == 3 && reflect.DeepEqual(status.Parts, expected))
		reconciled, readErr := uploadReconcile(bounded, client, command.ID)
		encoded, _ = json.Marshal(reconciled)
		var captured messagePartsStatus
		json.Unmarshal(encoded, &captured)
		add(label+"_owned_command_record_parts", readErr == nil && captured.CommitSeq == status.CommitSeq && reflect.DeepEqual(captured.Parts, expected))
		effects, effectErr := w.Effects()
		add(label+"_read_only_reconciliation_effect_one", effectErr == nil && effects[command.ID] == 1)
		checkWebMessageDelayedCapture(bounded, w, client, evidence, chatID, func(name string, ok bool) { add(label+"_"+name, ok) })
		stop()
	}
}

func checkWebMessageDelayedCapture(ctx context.Context, w *world.World, client *webClient, evidence *Evidence, chatID string, add func(string, bool)) {
	add("delayed_capture_fault_armed", w.ControlAction("/fault", map[string]any{"name": "outcome-before-capture"}) == nil)
	socket, err := client.socket()
	add("delayed_capture_socket", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] web delayed capture"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw}
	started := time.Now()
	result, err := uploadCommandReply(ctx, socket, command)
	add("delayed_capture_first_terminal_is_sent_without_client_resolve", err == nil && (result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered) && time.Since(started) < time.Second)
	var status messagePartsStatus
	encoded, _ := json.Marshal(result)
	json.Unmarshal(encoded, &status)
	expected := ""
	if len(status.Evidence.Parts) == 1 {
		aliases, problem := gatewayAliases(w)
		if problem == nil {
			expected = aliases[aliasKey(contracts.Messages, "messages.message", status.Evidence.Parts[0].NativeKey)]
		}
	}
	add("delayed_capture_exact_primary_and_part_identity", expected != "" && len(status.IDs) == 1 && status.IDs[0] == expected && len(status.Parts) == 1 && status.Parts[0].RecordID == expected && status.CommitSeq > 0)
	effects, problem := w.Effects()
	add("delayed_capture_effect_remains_one", problem == nil && effects[command.ID] == 1)
}
