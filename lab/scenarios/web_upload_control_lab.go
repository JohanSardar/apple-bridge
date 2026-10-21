//go:build lab

package scenarios

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webUploadCheckpointControl(response http.ResponseWriter, request *http.Request, w *world.World) bool {
	path := request.URL.Path
	if path != "/command-effect" && !strings.HasPrefix(path, "/replica-upload-checkpoint/") {
		return false
	}
	var input struct {
		UploadID  string `json:"upload_id,omitempty"`
		CommandID string `json:"command_id,omitempty"`
	}
	decoder := json.NewDecoder(io.LimitReader(request.Body, 4097))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&input) != nil || decoder.Decode(&struct{}{}) != io.EOF {
		http.Error(response, "invalid control request", 400)
		return true
	}
	if path == "/command-effect" {
		if !contracts.ValidCommandID(input.CommandID) || input.UploadID != "" {
			http.Error(response, "invalid command ID", 400)
			return true
		}
		effects, err := w.Effects()
		if err != nil {
			http.Error(response, "effect count unavailable", 500)
			return true
		}
		json.NewEncoder(response).Encode(map[string]int{"count": effects[input.CommandID]})
		return true
	}
	if !strings.HasPrefix(input.UploadID, "upload_") || !contracts.ValidCommandID(input.UploadID) || input.CommandID != "" {
		http.Error(response, "invalid upload ID", 400)
		return true
	}
	if len(w.Replicas) == 0 {
		http.Error(response, "replica unavailable", 409)
		return true
	}
	directory := filepath.Join(w.Replicas[len(w.Replicas)-1].Home, "upload-checkpoints", input.UploadID)
	switch strings.TrimPrefix(path, "/replica-upload-checkpoint/") {
	case "arm":
		if uploadLeaseFixturePath(w.Replicas[len(w.Replicas)-1].Home, input.UploadID) == "" {
			http.Error(response, "upload unavailable", 404)
			return true
		}
		if err := os.Mkdir(directory, 0700); err != nil {
			if os.IsNotExist(err) {
				os.MkdirAll(filepath.Dir(directory), 0700)
				err = os.Mkdir(directory, 0700)
			}
			if err != nil {
				http.Error(response, "checkpoint unavailable", 409)
				return true
			}
		}
		if err := os.WriteFile(filepath.Join(directory, "armed"), nil, 0600); err != nil {
			http.Error(response, "checkpoint unavailable", 500)
			return true
		}
		json.NewEncoder(response).Encode(map[string]any{"upload_id": input.UploadID, "state": "armed"})
	case "wait":
		for {
			raw, err := os.ReadFile(filepath.Join(directory, "paused"))
			if err == nil {
				response.Write(raw)
				return true
			}
			if _, err := os.Stat(directory); err != nil {
				http.Error(response, "checkpoint unavailable", 404)
				return true
			}
			select {
			case <-request.Context().Done():
				return true
			case <-time.After(20 * time.Millisecond):
			}
		}
	case "release":
		if _, err := os.Stat(filepath.Join(directory, "canceled")); err != nil {
			http.Error(response, "request is not canceled", 409)
			return true
		}
		if os.RemoveAll(directory) != nil {
			http.Error(response, "checkpoint unavailable", 500)
			return true
		}
		json.NewEncoder(response).Encode(map[string]any{"upload_id": input.UploadID, "state": "released", "canceled": true})
	default:
		http.NotFound(response, request)
	}
	return true
}

func uploadControlLabEnabled() bool { return true }
