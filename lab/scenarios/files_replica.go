package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runFilesReplica(ctx context.Context, s Scenario) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: "replica", At: time.Now().UTC(), Passed: true, Hosts: []string{"air", "ubuntu"}}
	add := func(name string, ok bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
		return ok
	}
	home, err := os.UserHomeDir()
	if !add("home", err == nil) {
		return e
	}
	root := filepath.Join(home, "Library/Mobile Documents/com~apple~CloudDocs/Bridge Test")
	folder := filepath.Join(root, "[ab-test "+e.RunID+"]")
	if !add("fixture_directory", os.MkdirAll(folder, 0700) == nil) {
		return e
	}
	name := "abtest" + strings.ReplaceAll(e.RunID, "_", "") + ".txt"
	content := []byte("Bridge Test synthetic replica payload")
	defer func() { _ = cleanupFilesReplica(folder, name, content) }()
	if !add("fixture_write", os.WriteFile(filepath.Join(folder, name), content, 0600) == nil) {
		return e
	}
	ctx = world.WithOptions(ctx, world.Options{GatewayDials: true, Scope: "files:read,files:write,system:read"})
	w, err := world.NewRealWithSetup(ctx, "replica", 1, world.NativeSetup{FilesystemTruth: true})
	if !add("ubuntu_replica_started", err == nil) {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
		}
		return e
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Replicas[0], Protocol: "rest"}
	err = w.Await(30*time.Second, func() bool {
		raw, _ := json.Marshal(map[string]string{"query": name, "field": "name"})
		var out contracts.QueryResult
		if client.Call(ctx, "files.search", contracts.Query{ID: contracts.NewID("qry"), Args: raw, DeadlineMS: 10000}, &out) != nil {
			return false
		}
		var result struct {
			Items []struct {
				Path string `json:"path"`
			} `json:"items"`
		}
		if json.Unmarshal(out.ResultJSON, &result) != nil {
			return false
		}
		for _, item := range result.Items {
			if item.Path == filepath.Join(folder, name) {
				return true
			}
		}
		return false
	})
	if !add("ubuntu_search", err == nil) {
		return e
	}
	raw, _ := json.Marshal(map[string]string{"root": root, "path": filepath.Join(filepath.Base(folder), name)})
	query := contracts.Query{ID: contracts.NewID("qry"), Args: raw, DeadlineMS: 15000}
	var out contracts.QueryResult
	err = client.Call(ctx, "files.read", query, &out)
	if !add("ubuntu_read_blob_reference", err == nil && len(out.Blobs) == 1) {
		return e
	}
	blob := out.Blobs[0]
	req, err := http.NewRequestWithContext(ctx, "GET", w.Replicas[0].URL+"/v1/blobs/"+blob.Digest+"?record="+query.ID, nil)
	if !add("blob_request", err == nil) {
		return e
	}
	req.Host = w.Replicas[0].Host
	req.Header.Set("Authorization", "Bearer "+w.Replicas[0].Token)
	response, err := http.DefaultClient.Do(req)
	if !add("ubuntu_blob_http", err == nil) {
		return e
	}
	bytes, readErr := io.ReadAll(io.LimitReader(response.Body, 1<<20))
	response.Body.Close()
	digest := sha256.Sum256(bytes)
	if !add("ubuntu_blob_digest", readErr == nil && response.StatusCode == 200 && hex.EncodeToString(digest[:]) == blob.Digest && string(bytes) == string(content)) {
		return e
	}
	sum := sha256.Sum256(content)
	raw, _ = json.Marshal(contracts.FileArgs{Root: root, Parent: filepath.Base(folder), Name: "created.txt", Content: base64.StdEncoding.EncodeToString(content), Digest: hex.EncodeToString(sum[:])})
	var command contracts.CommandRecord
	err = client.Call(ctx, "files.create", contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &command)
	if !add("ubuntu_create_observed", err == nil && command.State == contracts.CommandObserved) {
		return e
	}
	actual, err := os.ReadFile(filepath.Join(folder, "created.txt"))
	add("filesystem_truth_digest", err == nil && string(actual) == string(content))
	canary := filepath.Join(folder, "unowned.txt")
	preserved := []byte("Bridge Test preserve")
	if !add("cleanup_canary_created", os.WriteFile(canary, preserved, 0600) == nil) {
		return e
	}
	refused := cleanupFilesReplica(folder, name, content) != nil
	kept, readErr := os.ReadFile(canary)
	if !add("unowned_fixture_preserved", refused && readErr == nil && string(kept) == string(preserved)) {
		return e
	}
	if !add("cleanup_canary_removed", os.Remove(canary) == nil) {
		return e
	}
	add("owned_fixture_cleanup", cleanupFilesReplica(folder, name, content) == nil)
	return e
}

func cleanupFilesReplica(folder, name string, content []byte) error {
	info, err := os.Lstat(folder)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	if !info.IsDir() || filepath.Base(filepath.Dir(folder)) != "Bridge Test" || !strings.HasPrefix(filepath.Base(folder), "[ab-test ") {
		return contracts.ErrInvalid
	}
	entries, err := os.ReadDir(folder)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if entry.Name() != name && entry.Name() != "created.txt" || !entry.Type().IsRegular() {
			return contracts.ErrInvalid
		}
		info, err := entry.Info()
		if err != nil || info.Size() != int64(len(content)) {
			return contracts.ErrInvalid
		}
		file, err := os.Open(filepath.Join(folder, entry.Name()))
		if err != nil {
			return err
		}
		data, err := io.ReadAll(io.LimitReader(file, int64(len(content)+1)))
		file.Close()
		if err != nil || string(data) != string(content) {
			return contracts.ErrInvalid
		}
	}
	return os.RemoveAll(folder)
}
