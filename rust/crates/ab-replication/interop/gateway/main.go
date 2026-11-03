package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func run() error {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	home, err := os.MkdirTemp("", "ab-gc-interop-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(home)
	gateway := exec.CommandContext(ctx, os.Args[1], filepath.Join(home, "gateway"))
	gateway.Stderr = os.Stderr
	output, err := gateway.StdoutPipe()
	if err != nil {
		return err
	}
	if err = gateway.Start(); err != nil {
		return err
	}
	defer func() { gateway.Process.Kill(); gateway.Wait() }()
	scanner := bufio.NewScanner(output)
	if !scanner.Scan() {
		return fmt.Errorf("missing pairing code")
	}
	code := scanner.Text()
	if !scanner.Scan() {
		return fmt.Errorf("missing Rust pairing code")
	}
	rustCode := scanner.Text()
	rustHome := filepath.Join(home, "rust")
	rustClient := exec.CommandContext(ctx, os.Args[2], rustHome, rustCode)
	rustClient.Stderr = os.Stderr
	if err = rustClient.Start(); err != nil {
		return err
	}
	defer func() { rustClient.Process.Kill(); rustClient.Wait() }()
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "status.json"))
		var status struct {
			Snapshots uint64 `json:"snapshots"`
		}
		if json.Unmarshal(bytes, &status) == nil && status.Snapshots == 1 {
			break
		}
		if ctx.Err() != nil {
			return fmt.Errorf("Rust snapshot timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	authority, err := trust.Open(filepath.Join(home, "go"))
	if err != nil {
		return err
	}
	if err = authority.Join(ctx, code); err != nil {
		return err
	}
	store, err := state.Open(filepath.Join(home, "go"))
	if err != nil {
		return err
	}
	defer store.Close()
	blobStore, err := blobs.Open(filepath.Join(home, "go"), blobs.Config{})
	if err != nil {
		return err
	}
	defer blobStore.Close()
	client := &replication.Client{Store: store, Authority: authority, Blobs: blobStore}
	follow, stop := context.WithCancel(ctx)
	defer stop()
	done := make(chan error, 1)
	go func() { done <- client.Follow(follow) }()
	for client.Status().Snapshots != 1 || store.Head().Sequence != 2 {
		select {
		case <-ctx.Done():
			return fmt.Errorf("snapshot timeout: %+v", client.Status())
		case <-time.After(10 * time.Millisecond):
		}
	}
	var rustState struct {
		Cursor struct {
			Sequence uint64 `json:"sequence"`
			Revision uint64 `json:"subscription_revision"`
		} `json:"cursor"`
		Digest string `json:"digest"`
	}
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "state.json"))
		if json.Unmarshal(bytes, &rustState) == nil && rustState.Cursor.Sequence == 2 {
			break
		}
		if ctx.Err() != nil {
			return fmt.Errorf("Rust live timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	for _, peerHome := range []string{rustHome, filepath.Join(home, "go")} {
		digest := trust.Fingerprint([]byte("fixture"))
		for {
			bytes, _ := os.ReadFile(filepath.Join(peerHome, "blobs", "sha256", digest[:2], digest[2:]))
			if string(bytes) == "fixture" {
				break
			}
			if ctx.Err() != nil {
				return fmt.Errorf("blob timeout for %s", filepath.Base(peerHome))
			}
			time.Sleep(10 * time.Millisecond)
		}
	}
	stop()
	<-done
	follow, stop = context.WithCancel(ctx)
	defer stop()
	go func() { done <- client.Follow(follow) }()
	for client.Status().Resumes < 1 {
		if ctx.Err() != nil {
			return fmt.Errorf("Go resume timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	enrollment, err := authority.Enrollment()
	if err != nil {
		return err
	}
	gatewayAuthority, err := trust.Open(filepath.Join(home, "gateway"))
	if err != nil {
		return err
	}
	rustAuthority, err := trust.Open(rustHome)
	if err != nil {
		return err
	}
	rustEnrollment, err := rustAuthority.Enrollment()
	if err != nil {
		return err
	}
	rustClient.Process.Kill()
	rustClient.Wait()
	rustClient = exec.CommandContext(ctx, os.Args[2], rustHome, "-")
	rustClient.Stderr = os.Stderr
	if err = rustClient.Start(); err != nil {
		return err
	}
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "status.json"))
		var status struct {
			Resumes uint64 `json:"resumes"`
		}
		if json.Unmarshal(bytes, &status) == nil && status.Resumes > 0 {
			break
		}
		if ctx.Err() != nil {
			return fmt.Errorf("Rust resume timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	snapshot, err := store.FreezeSnapshot(ctx, trust.AllVisibility(), 1)
	if err != nil {
		return err
	}
	digest := snapshot.Digest
	snapshot.Close()
	if digest != rustState.Digest {
		return fmt.Errorf("digest mismatch: Go %s Rust %s", digest, rustState.Digest)
	}
	if err = gatewayAuthority.ChangeScope(enrollment.ReplicaID, []contracts.Grant{"messages:read"}); err != nil {
		return err
	}
	if err = gatewayAuthority.ChangeScope(rustEnrollment.ReplicaID, []contracts.Grant{"messages:read"}); err != nil {
		return err
	}
	for client.Status().Snapshots < 2 {
		if ctx.Err() != nil {
			return fmt.Errorf("Go scope resnapshot timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "status.json"))
		var status struct {
			Snapshots uint64 `json:"snapshots"`
		}
		if json.Unmarshal(bytes, &status) == nil && status.Snapshots >= 1 {
			break
		}
		if ctx.Err() != nil {
			return fmt.Errorf("Rust scope resnapshot timeout")
		}
		time.Sleep(10 * time.Millisecond)
	}
	scoped, err := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{})
	if err != nil {
		return err
	}
	if len(scoped.Records) != 0 {
		return fmt.Errorf("Go retained records after narrowing scope")
	}
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "state.json"))
		if json.Unmarshal(bytes, &rustState) == nil && rustState.Cursor.Revision == 2 && rustState.Digest == state.Digest(nil) {
			break
		}
		if ctx.Err() != nil {
			return fmt.Errorf("Rust retained records after narrowing scope")
		}
		time.Sleep(10 * time.Millisecond)
	}
	start := time.Now()
	if err = gatewayAuthority.Revoke(enrollment.ReplicaID); err != nil {
		return err
	}
	for client.Status().State != "disconnected" && client.Status().State != "connecting" {
		if time.Since(start) > time.Second {
			return fmt.Errorf("revocation exceeded 1 second")
		}
		time.Sleep(time.Millisecond)
	}
	goRevocation := time.Since(start).Milliseconds()
	start = time.Now()
	if err = gatewayAuthority.Revoke(rustEnrollment.ReplicaID); err != nil {
		return err
	}
	for {
		bytes, _ := os.ReadFile(filepath.Join(rustHome, "status.json"))
		var status struct {
			State string `json:"state"`
		}
		if json.Unmarshal(bytes, &status) == nil && (status.State == "disconnected" || status.State == "connecting") {
			break
		}
		if time.Since(start) > time.Second {
			return fmt.Errorf("Rust revocation exceeded 1 second")
		}
		time.Sleep(time.Millisecond)
	}
	result := map[string]any{"gateway": "rust-fixture", "replica": "go", "paired": true, "rust_paired_snapshot_live_resume_scope": true, "mixed_digest": digest, "go_resumes": client.Status().Resumes, "snapshots": client.Status().Snapshots, "go_revocation_ms": goRevocation, "rust_revocation_ms": time.Since(start).Milliseconds(), "blobs_both": true, "cursor": store.Head()}
	return json.NewEncoder(os.Stdout).Encode(result)
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
