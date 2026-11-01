package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func notesFolderRecovery(truth func(string, map[string]any) (map[string]any, error), helper string) func(string, map[string]any) (map[string]any, error) {
	return func(verb string, request map[string]any) (map[string]any, error) {
		if verb != "fixture" || request["action"] != "cleanup_created_folders" {
			return truth(verb, request)
		}
		root, problem := os.MkdirTemp("", "ab-test-notes-cleanup-")
		if problem != nil {
			return nil, problem
		}
		defer os.RemoveAll(root)
		home, problem := os.UserHomeDir()
		if problem != nil {
			return nil, problem
		}
		input := filepath.Join(root, "request.json")
		output := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", contracts.NewID("notes-owned-cleanup")+".json")
		defer os.Remove(output)
		raw, problem := json.Marshal(request)
		if problem != nil {
			return nil, problem
		}
		if problem = os.WriteFile(input, raw, 0600); problem != nil {
			return nil, problem
		}
		ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
		defer cancel()
		problem = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", helper, "--args", "notes-owned-cleanup", "--request", input, "--report", output).Run()
		if problem != nil {
			return nil, fmt.Errorf("Notes cleanup UI: helper_unavailable")
		}
		raw, problem = os.ReadFile(output)
		var result struct {
			Passed bool   `json:"passed"`
			Stage  string `json:"stage"`
		}
		if problem != nil || json.Unmarshal(raw, &result) != nil {
			return nil, fmt.Errorf("Notes cleanup UI: report_unavailable")
		}
		if !result.Passed {
			return nil, fmt.Errorf("Notes cleanup UI: %s", result.Stage)
		}
		folders, _ := request["folders"].([]map[string]any)
		ids := make([]string, 0, len(folders))
		for _, folder := range folders {
			id, _ := folder["id"].(string)
			if id == "" {
				return nil, fmt.Errorf("Notes cleanup UI: captured_identity_missing")
			}
			ids = append(ids, id)
		}
		proof, problem := truth("observe", map[string]any{"source": "notes", "verify_deleted_ids": ids})
		if problem != nil || proof["status"] != "verified" || proof["deletion_verified"] != true {
			return nil, fmt.Errorf("Notes cleanup UI: independent_deletion_unverified")
		}
		return map[string]any{"status": "verified"}, nil
	}
}
