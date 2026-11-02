package main

import (
	"context"
	"crypto/sha256"
	"crypto/x509"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func run(ctx context.Context) error {
	if len(os.Args) > 1 && os.Args[1] == "simapple" {
		f := flag.NewFlagSet("simapple", flag.ContinueOnError)
		s := f.String("socket", "", "")
		k := f.String("secret-file", "", "")
		c := f.String("control", "", "")
		if err := f.Parse(os.Args[2:]); err != nil {
			return err
		}
		return simapple.Run(ctx, simapple.Config{Socket: *s, SecretFile: *k, Control: *c})
	}
	w, err := world.New(ctx, "sim", 0, 3, world.BlobOptions{Slow: true, Scope: "messages:read,reminders:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	var cfg world.Config
	b, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err != nil {
		return err
	}
	if err = json.Unmarshal(b, &cfg); err != nil {
		return err
	}
	dial := ""
	if os.Getenv("AB_GATEWAY_DIALS") == "1" {
		listener, e := net.Listen("tcp", "127.0.0.1:0")
		if e != nil {
			return e
		}
		dial = listener.Addr().String()
		listener.Close()
	}
	pairArgs := []string{"pair", "--home", w.Gateway.Home, "--peer-addr", cfg.PeerAddress, "--scope", "messages:read,reminders:read,system:read"}
	if dial != "" {
		pairArgs = append(pairArgs, "--dial-replica", dial)
	}
	cmd := exec.CommandContext(ctx, "bin/bridge", pairArgs...)
	code, err := cmd.Output()
	if err != nil {
		return err
	}
	home := filepath.Join(w.Dir, "rust-probe")
	binary := os.Getenv("AB_FEED_PROBE")
	if binary == "" {
		return fmt.Errorf("AB_FEED_PROBE required")
	}
	var process *exec.Cmd
	start := func(pair string) error {
		args := []string{home}
		if pair != "" || dial != "" {
			args = append(args, pair)
		}
		if dial != "" {
			args = append(args, dial)
		}
		process = exec.CommandContext(ctx, binary, args...)
		process.Stdout = os.Stderr
		process.Stderr = os.Stderr
		return process.Start()
	}
	stop := func() {
		if process != nil {
			process.Process.Kill()
			process.Wait()
			process = nil
		}
	}
	defer stop()
	if err = start(strings.TrimSpace(string(code))); err != nil {
		return err
	}
	await := func(predicate func() bool) error { return w.Await(20*time.Second, predicate) }
	var probe struct {
		PartialBatches uint64                      `json:"partial_batches"`
		Cursor         contracts.Cursor            `json:"cursor"`
		Records        map[string]contracts.Record `json:"records"`
	}
	read := func() bool {
		raw, e := os.ReadFile(filepath.Join(home, "probe-state.json"))
		probe.Records = nil
		return e == nil && json.Unmarshal(raw, &probe) == nil
	}
	if err = await(func() bool { return read() && len(probe.Records) >= 4 }); err != nil {
		return fmt.Errorf("snapshot: %w", err)
	}
	local, err := trust.Open(home)
	if err != nil {
		return fmt.Errorf("Go reopen: %w", err)
	}
	token, err := os.ReadFile(filepath.Join(home, "probe-token"))
	if err != nil {
		return err
	}
	if _, err = local.AuthenticateToken(string(token)); err != nil {
		return fmt.Errorf("Go token: %w", err)
	}
	delegation, err := os.ReadFile(filepath.Join(home, "probe-delegation"))
	if err != nil {
		return err
	}
	gatewayTrust, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return err
	}
	cert, err := x509.ParseCertificate(local.Certificate.Certificate[0])
	if err != nil {
		return err
	}
	if _, err = gatewayTrust.VerifyDelegation(cert, string(delegation), "interop", trust.Fingerprint([]byte("interop"))); err != nil {
		return fmt.Errorf("Go delegation: %w", err)
	}
	if err = await(func() bool { _, e := os.Stat(filepath.Join(home, "probe-commands.json")); return e == nil }); err != nil {
		return fmt.Errorf("command exchange: %w", err)
	}
	initial := probe.Cursor.Sequence
	capture := contracts.Capture{Source: contracts.Reminders}
	for i := 0; i < 160; i++ {
		key := fmt.Sprintf("atomic-%d", i)
		data := map[string]any{"title": "[ab-test atomic] " + key}
		if err = w.ControlAction("/external-change", map[string]any{"key": key, "data": data, "omit_capture": true}); err != nil {
			return err
		}
		raw, _ := json.Marshal(data)
		capture.Observations = append(capture.Observations, contracts.Observation{Kind: "upsert", Collection: "reminders.reminder", NativeKey: key, Data: raw, ObservedAt: time.Now().UTC()})
	}
	if err = w.ControlAction("/capture-atomic", capture); err != nil {
		return err
	}
	if err = w.FlushCaptures(10 * time.Second); err != nil {
		return err
	}
	if err = await(func() bool { return read() && probe.Cursor.Sequence > initial && probe.PartialBatches > 0 }); err != nil {
		return fmt.Errorf("live: %w", err)
	}
	if probe.PartialBatches == 0 {
		return fmt.Errorf("no partial capture observed")
	}
	before := probe.Cursor.Sequence
	stop()
	if err = w.ControlAction("/seed", map[string]int{"count": 2}); err != nil {
		return err
	}
	if err = w.FlushCaptures(10 * time.Second); err != nil {
		return err
	}
	if err = start(""); err != nil {
		return err
	}
	if err = await(func() bool {
		var s struct {
			Resumes int `json:"resumes"`
		}
		b, e := os.ReadFile(filepath.Join(home, "probe-status.json"))
		return e == nil && json.Unmarshal(b, &s) == nil && s.Resumes > 0 && read() && probe.Cursor.Sequence > before
	}); err != nil {
		return fmt.Errorf("resume: %w", err)
	}
	var fixture map[string]any
	if err = w.ControlResult("/blob-fixture", map[string]any{"size": 2 << 20}, &fixture); err != nil {
		return err
	}
	if err = w.FlushCaptures(10 * time.Second); err != nil {
		return err
	}
	if err = await(func() bool {
		parts, _ := filepath.Glob(filepath.Join(home, "blobs", "staging", "*.part"))
		for _, p := range parts {
			info, e := os.Stat(p)
			if e == nil && info.Size() > 0 && info.Size() < 2<<20 {
				return true
			}
		}
		return false
	}); err != nil {
		return fmt.Errorf("partial blob: %w", err)
	}
	stop()
	if err = start(""); err != nil {
		return err
	}
	var digest string
	if err = await(func() bool {
		if !read() {
			return false
		}
		for _, r := range probe.Records {
			for _, ref := range contracts.BlobReferences(r.Data) {
				digest = ref.Digest
				b, e := os.ReadFile(filepath.Join(home, "blobs", "sha256", digest[:2], digest[2:]))
				if e == nil && fmt.Sprintf("%x", sha256.Sum256(b)) == digest {
					return true
				}
			}
		}
		return false
	}); err != nil {
		return fmt.Errorf("blob: %w", err)
	}
	var progress struct {
		Entries map[string]struct {
			ResumedBytes int64 `json:"resumed_bytes"`
		} `json:"entries"`
	}
	manifest, e := os.ReadFile(filepath.Join(home, "blobs", "manifest.json"))
	if e != nil {
		return e
	}
	if e = json.Unmarshal(manifest, &progress); e != nil {
		return e
	}
	if progress.Entries[digest].ResumedBytes <= 0 {
		return fmt.Errorf("blob did not resume")
	}
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "messages:read,reminders:read,system:read")
	if err != nil {
		return err
	}
	gateway, err := w.Records(endpoint)
	if err != nil {
		return err
	}
	if err = await(func() bool {
		read()
		rs := []contracts.Record{}
		for _, r := range probe.Records {
			if !r.Deleted {
				rs = append(rs, r)
			}
		}
		return state.Digest(rs) == state.Digest(gateway)
	}); err != nil {
		return fmt.Errorf("digest: %w", err)
	}
	enrollment, err := local.Enrollment()
	if err != nil {
		return err
	}
	if err = gatewayTrust.ChangeScope(enrollment.ReplicaID, []contracts.Grant{"reminders:read", "system:read"}); err != nil {
		return err
	}
	if err = await(func() bool {
		if !read() || probe.Cursor.SubscriptionRevision < 2 {
			return false
		}
		for _, r := range probe.Records {
			if r.Source == contracts.Messages {
				return false
			}
		}
		return true
	}); err != nil {
		return fmt.Errorf("scope reset: %w", err)
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"impl": "rust", "state": "sqlite", "scope_reset": true, "gateway_dials": dial != "", "go_reopen": true, "go_token": true, "go_delegation": true, "feed_commands": true, "direct_mtls_commands": true, "pairing": true, "snapshot": true, "live_batches": true, "capture_atomic": true, "resume": true, "blob": digest, "blob_resume": true, "snapshot_digest": state.Digest(gateway), "records": len(gateway)})
}
func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	if err := run(ctx); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
