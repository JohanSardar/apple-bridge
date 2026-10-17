package runtime

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

type gatewayClockSample struct {
	Gateway  time.Time `json:"gateway"`
	Received time.Time `json:"received"`
}

func linuxEncryption(ctx context.Context, home string) string {
	output, err := exec.CommandContext(ctx, "findmnt", "-n", "-o", "MAJ:MIN", "-T", home).Output()
	if err != nil {
		return "unverified"
	}
	device := strings.TrimSpace(string(output))
	if device == "" || strings.ContainsAny(device, "/\\ \n") {
		return "unverified"
	}
	root, err := filepath.EvalSymlinks(filepath.Join("/sys/dev/block", device))
	if err != nil {
		return "unverified"
	}
	seen := map[string]bool{}
	var encrypted func(string) bool
	encrypted = func(path string) bool {
		resolved, err := filepath.EvalSymlinks(path)
		if err != nil || seen[resolved] {
			return false
		}
		seen[resolved] = true
		uuid, _ := os.ReadFile(filepath.Join(resolved, "dm", "uuid"))
		if strings.HasPrefix(strings.ToUpper(strings.TrimSpace(string(uuid))), "CRYPT-") {
			return true
		}
		slaves, _ := filepath.Glob(filepath.Join(resolved, "slaves", "*"))
		for _, slave := range slaves {
			if encrypted(slave) {
				return true
			}
		}
		if _, err := os.Stat(filepath.Join(resolved, "partition")); err == nil {
			return encrypted(filepath.Dir(resolved))
		}
		return false
	}
	if encrypted(root) {
		return "enabled"
	}
	output, err = exec.CommandContext(ctx, "lsblk", "--inverse", "--json", "--output", "TYPE", filepath.Join("/dev/block", device)).Output()
	if err != nil {
		return "unverified"
	}
	type block struct {
		Type     string            `json:"type"`
		Children []json.RawMessage `json:"children"`
	}
	var result struct {
		Devices []json.RawMessage `json:"blockdevices"`
	}
	if json.Unmarshal(output, &result) != nil || len(result.Devices) == 0 {
		return "unverified"
	}
	var crypt func(json.RawMessage) bool
	crypt = func(raw json.RawMessage) bool {
		var b block
		if json.Unmarshal(raw, &b) != nil {
			return false
		}
		if b.Type == "crypt" {
			return true
		}
		for _, child := range b.Children {
			if crypt(child) {
				return true
			}
		}
		return false
	}
	for _, b := range result.Devices {
		if crypt(b) {
			return "enabled"
		}
	}
	return "warning_not_enabled"
}
