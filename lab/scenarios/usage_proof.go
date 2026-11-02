package scenarios

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"path/filepath"
)

func writeUsageProof(result contracts.ReadResult, reference map[string]map[string]float64) error {
	directory := os.Getenv("APPLE_BRIDGE_USAGE_PROOF_DIR")
	if directory == "" {
		return nil
	}
	keys := map[string]string{}
	for _, alias := range result.Aliases {
		if alias.Collection == "usage.app_day" {
			sum := sha256.Sum256([]byte(alias.NativeKey))
			keys[alias.RecordID] = hex.EncodeToString(sum[:])
		}
	}
	names := map[string]map[string]any{}
	rows := []map[string]any{}
	for _, record := range result.Records {
		if record.Collection != "usage.device" {
			continue
		}
		var data map[string]any
		if err := json.Unmarshal(record.Data, &data); err != nil {
			return err
		}
		names[record.ID] = data
	}
	for _, record := range result.Records {
		if record.Collection != "usage.app_day" {
			continue
		}
		var data map[string]any
		if err := json.Unmarshal(record.Data, &data); err != nil {
			return err
		}
		id, _ := data["device_id"].(string)
		device := names[id]
		rows = append(rows, map[string]any{"device": device["name"], "local": device["is_local"], "name_status": device["name_status"], "day": data["day"], "bundle_id": data["bundle_id"], "seconds": data["seconds_estimated"], "clipped": data["clipped_seconds"], "raw_seconds": reference[keys[record.ID]]["seconds_estimated"]})
	}
	if err := os.MkdirAll(directory, 0700); err != nil {
		return err
	}
	raw, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(directory, "real-counts.json"), raw, 0600)
}
