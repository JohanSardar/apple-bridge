package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
	"io"
	"net/http"
	"strings"
	"time"
)

func argsFiles(args map[string]any) []contracts.MessageFile {
	files, _ := args["files"].([]contracts.MessageFile)
	return files
}

func messagesObservedAt(observer *watcher, text string, fromMe bool) (time.Time, bool) {
	observer.mu.Lock()
	defer observer.mu.Unlock()
	var earliest time.Time
	for _, seen := range observer.changes {
		if seen.Change.Record.Collection != "messages.message" {
			continue
		}
		var data struct {
			Text   string `json:"text"`
			FromMe bool   `json:"is_from_me"`
		}
		json.Unmarshal(seen.Change.Record.Data, &data)
		if data.Text == text && data.FromMe == fromMe && (earliest.IsZero() || seen.At.Before(earliest)) {
			earliest = seen.At
		}
	}
	return earliest, !earliest.IsZero()
}
func saveIncomingEchoEvidence(e Evidence) error {
	if e.Scenario != "MSG-SEND-SELF-TEXT" || (e.Level != "real" && e.Level != "replica") {
		return nil
	}
	incoming := e
	incoming.Scenario = "MSG-INCOMING"
	incoming.Assertions = []Assertion{}
	incoming.Latency = map[string]Latency{}
	incoming.Metrics = map[string]any{"driven_by": "self_send_incoming_echo", "source_scenario": "MSG-SEND-SELF-TEXT", "maximum_sends": 1, "chat_db_poll_interval_ms": 10}
	for _, a := range e.Assertions {
		if strings.HasPrefix(a.Name, "incoming_echo_") {
			incoming.Assertions = append(incoming.Assertions, a)
		}
	}
	for name, value := range e.Latency {
		if strings.HasPrefix(name, "echo_chat_db_") {
			incoming.Latency[name] = value
		}
	}
	incoming.Passed = len(incoming.Assertions) > 0 && len(incoming.Latency) > 0 && incoming.Status == ""
	for _, a := range incoming.Assertions {
		incoming.Passed = incoming.Passed && a.Passed
	}
	return Save(incoming)
}

func messagesAttachmentBytes(ctx context.Context, w *world.World, endpoint world.Endpoint, id string, expected []byte, diagnostics ...map[string]any) bool {
	diagnostic := map[string]any{}
	if len(diagnostics) > 0 {
		diagnostic = diagnostics[0]
	}
	return w.Await(10*time.Second, func() bool {
		var result contracts.ReadResult
		err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.attachment", ID: id}, &result)
		if err != nil || len(result.Records) != 1 {
			return false
		}
		var data struct {
			Blob struct {
				Digest string `json:"digest"`
				State  string `json:"state"`
			} `json:"blob"`
		}
		if json.Unmarshal(result.Records[0].Data, &data) != nil || data.Blob.Digest == "" {
			return false
		}
		status, bytes, err := blobBytes(ctx, endpoint, data.Blob.Digest, id)
		diagnostic["http_status"] = status
		diagnostic["record_blob_state"] = data.Blob.State
		if progress, problem := blobProgress(ctx, endpoint); problem == nil {
			entry := progress.Entries[data.Blob.Digest]
			diagnostic["local_blob_state"] = entry.State
			diagnostic["local_blob_reason"] = entry.Reason
		}
		return err == nil && status == 200 && string(bytes) == string(expected)
	}) == nil
}

func appendDiagnostic(previous any, next map[string]any) []map[string]any {
	values, _ := previous.([]map[string]any)
	return append(values, next)
}

func messageSyntheticFile(tag string) []byte {
	const size = 5 * 1024 * 1024
	part := []byte(tag + " synthetic attachment\n")
	return bytes.Repeat(part, size/len(part)+1)[:size]
}
func stageMessageFile(ctx context.Context, endpoint world.Endpoint, name string, data []byte) (contracts.MessageFile, error) {
	req, err := http.NewRequestWithContext(ctx, "POST", endpoint.URL+"/v1/blobs", bytes.NewReader(data))
	if err != nil {
		return contracts.MessageFile{}, err
	}
	req.Host = endpoint.Host
	req.Header.Set("Authorization", "Bearer "+endpoint.Token)
	response, err := world.HTTP.Do(req)
	if err != nil {
		return contracts.MessageFile{}, err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		io.Copy(io.Discard, response.Body)
		return contracts.MessageFile{}, contracts.ErrUnavailable
	}
	var upload struct {
		UploadID string `json:"upload_id"`
	}
	err = json.NewDecoder(response.Body).Decode(&upload)
	if upload.UploadID == "" {
		return contracts.MessageFile{}, contracts.ErrInvalid
	}
	return contracts.MessageFile{Name: name, UploadID: upload.UploadID}, err
}

func submitInteractiveMessage(ctx context.Context, conn *websocket.Conn, command contracts.Command) (contracts.CommandRecord, error) {
	bounded, cancel := context.WithTimeout(ctx, 25*time.Second)
	defer cancel()
	if err := wsjson.Write(bounded, conn, map[string]any{"t": "cmd", "id": command.ID, "op": command.Op, "args": command.Args}); err != nil {
		return contracts.CommandRecord{}, err
	}
	for {
		var response struct {
			T       string                  `json:"t"`
			Command contracts.CommandRecord `json:"command"`
			Error   *contracts.Error        `json:"error"`
		}
		if err := wsjson.Read(bounded, conn, &response); err != nil {
			return contracts.CommandRecord{}, err
		}
		if response.T == "cmd_status" && response.Command.Command.ID == command.ID && response.Command.Disposition != "" {
			if response.Error != nil {
				return response.Command, response.Error
			}
			return response.Command, nil
		}
	}
}
