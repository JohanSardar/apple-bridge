package scenarios

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadMissingPart(w *world.World, c *webClient, add func(string, bool)) {
	for _, mode := range []string{"missing_append", "short_append", "missing_commit"} {
		data := bytes.Repeat([]byte("Bridge Test lost upload bytes\n"), 80000)
		status, out := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(len(data))})
		add(mode+"_begun_without_optional_digest", status == 200 && out.ID != "")
		if out.ID == "" {
			continue
		}
		status, part := uploadRequest(c, "POST", "upload", bytes.NewReader(data[:1<<20]), map[string]string{"Upload-ID": out.ID, "Upload-Offset": "0"})
		add(mode+"_prefix_accepted", status == 200 && part.Offset == 1<<20)
		paths, _ := filepath.Glob(filepath.Join(w.Gateway.Home, "blobs/upload-leases/*.json"))
		fault := false
		for _, path := range paths {
			raw, _ := os.ReadFile(path)
			var lease map[string]any
			if json.Unmarshal(raw, &lease) != nil || lease["upload_id"] != out.ID {
				continue
			}
			partPath := strings.TrimSuffix(path, ".json") + ".part"
			var err error
			if mode == "short_append" {
				err = os.Truncate(partPath, 100)
			} else {
				err = os.Remove(partPath)
			}
			if mode == "missing_commit" && err == nil {
				lease["size"] = part.Offset
				raw, _ = json.Marshal(lease)
				err = os.WriteFile(path, raw, 0600)
			}
			fault = err == nil
			break
		}
		add(mode+"_fault_injected", fault)
		before, err := uploadCASCount(w.Gateway.Home)
		add(mode+"_cas_inventory", err == nil)
		tail := data[1<<20:]
		if mode == "missing_commit" {
			tail = nil
		}
		status, _ = uploadRequest(c, "POST", "upload", bytes.NewReader(tail), map[string]string{"Upload-ID": out.ID, "Upload-Offset": fmt.Sprint(part.Offset)})
		add(mode+"_refused_with_conflict", status == 409)
		after, err := uploadCASCount(w.Gateway.Home)
		add(mode+"_no_blob_staged", err == nil && after == before)
		uploadRequest(c, "DELETE", "upload/"+out.ID, nil, nil)
	}
}

func uploadCASCount(home string) (int, error) {
	count := 0
	err := filepath.WalkDir(filepath.Join(home, "blobs/sha256"), func(_ string, entry fs.DirEntry, err error) error {
		if os.IsNotExist(err) {
			return nil
		}
		if err != nil {
			return err
		}
		if entry.Type().IsRegular() {
			count++
		}
		return nil
	})
	return count, err
}
