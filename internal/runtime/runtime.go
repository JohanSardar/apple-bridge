package runtime

import (
	"context"
	"crypto/rand"
	"crypto/tls"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/frontdoor"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Config struct {
	BlobCap     int64 `json:"blob_cap,omitempty"`
	BlobGraceMS int64 `json:"blob_grace_ms,omitempty"`

	Role              string `json:"role"`
	Home              string `json:"home"`
	PeerAddress       string `json:"peer_address"`
	APIAddress        string `json:"api_address"`
	Socket            string `json:"socket"`
	HelperApp         string `json:"helper_app,omitempty"`
	HelperLabCommands bool   `json:"helper_lab_commands,omitempty"`
}

func LoadConfig(home string) (Config, error) {
	var c Config
	b, err := os.ReadFile(filepath.Join(home, "runtime.json"))
	if err != nil {
		return c, err
	}
	err = json.Unmarshal(b, &c)
	return c, err
}

func Start(ctx context.Context, cfg Config) error {
	if cfg.Role != "gateway" && cfg.Role != "replica" {
		return contracts.ErrInvalid
	}
	syscall.Umask(0077)
	if err := os.MkdirAll(cfg.Home, 0700); err != nil {
		return err
	}
	if err := os.Chmod(cfg.Home, 0700); err != nil {
		return err
	}
	lock, err := os.OpenFile(filepath.Join(cfg.Home, "lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return contracts.ErrConflict
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	authority, err := trust.Open(cfg.Home)
	if err != nil {
		return err
	}
	store, err := state.Open(cfg.Home)
	if err != nil {
		return err
	}
	defer store.Close()
	blobCap := cfg.BlobCap
	if cfg.Role == "gateway" && blobCap == 0 {
		blobCap = -1
	}
	blobStore, err := blobs.Open(cfg.Home, blobs.Config{Cap: blobCap, Grace: time.Duration(cfg.BlobGraceMS) * time.Millisecond})
	if err != nil {
		return err
	}
	store.BlobProtection = blobStore.Protect
	if err = store.ProtectBlobs(ctx); err != nil {
		return err
	}

	if cfg.Role == "gateway" {
		if err = store.InitCommands(ctx); err != nil {
			return err
		}
	}
	os.Setenv("APPLE_BRIDGE_HOME", cfg.Home)
	if cfg.APIAddress == "" {
		cfg.APIAddress = "127.0.0.1:7780"
	}
	host, _, err := net.SplitHostPort(cfg.APIAddress)
	if err != nil || host != "127.0.0.1" {
		return contracts.ErrInvalid
	}
	api, err := net.Listen("tcp", cfg.APIAddress)
	if err != nil {
		return err
	}
	defer api.Close()
	cfg.APIAddress = api.Addr().String()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	failures := make(chan error, 4)
	go func() {
		tick := time.NewTicker(100 * time.Millisecond)
		defer tick.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-tick.C:
				if store.ProtectBlobs(ctx) == nil {
					blobStore.Collect(ctx)
				}
			}
		}
	}()

	if err = serveBlobLab(ctx, cfg.Home, store, blobStore); err != nil {
		return err
	}
	service := &operations.Service{State: store}
	service.LocalStatus = func() contracts.ReplicaStatus {
		return contracts.ReplicaStatus{State: "gateway", Cursor: store.Head(), Blobs: blobStore.Progress()}
	}
	if cfg.Role == "gateway" {
		if cfg.PeerAddress == "" {
			cfg.PeerAddress = "127.0.0.1:7443"
		}
		if cfg.Socket == "" {
			cfg.Socket = filepath.Join(cfg.Home, "run", "peer.sock")
		}
		secretPath := filepath.Join(cfg.Home, "apple-secret")
		secret, err := os.ReadFile(secretPath)
		if os.IsNotExist(err) {
			secret = make([]byte, 32)
			if _, err = rand.Read(secret); err == nil {
				err = os.WriteFile(secretPath, secret, 0600)
			}
		}
		if err != nil {
			return err
		}
		if len(secret) != 32 {
			return contracts.ErrCorrupt
		}
		peer, err := tls.Listen("tcp", cfg.PeerAddress, &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{authority.Certificate}, ClientAuth: tls.RequestClientCert})
		if err != nil {
			return err
		}
		defer peer.Close()
		cfg.PeerAddress = peer.Addr().String()
		commands := &applepeer.Commands{}
		audit, err := os.OpenFile(filepath.Join(cfg.Home, "commands-audit.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
		if err != nil {
			return err
		}
		defer audit.Close()
		var auditMu sync.Mutex
		gateway := &operations.LocalGateway{State: store, Authority: authority, Peer: commands, Context: ctx, Audit: func(record contracts.CommandRecord) {
			auditMu.Lock()
			defer auditMu.Unlock()
			json.NewEncoder(audit).Encode(map[string]any{"command_id": record.Command.ID, "operation": record.Command.Op, "request_hash": record.RequestHash, "argument_bytes": len(record.Command.Args), "state": record.State, "at": time.Now().UTC()})
		}}
		if err = gateway.Recover(ctx); err != nil {
			return err
		}
		service.Target = gateway
		feed := &replication.Server{Store: store, Authority: authority, Blobs: blobStore}
		mux := http.NewServeMux()
		mux.Handle("/v1/pair", authority.PairHandler())
		mux.Handle("/v1/feed", feed.Handler())
		mux.Handle("/v1/commands", gateway.Handler())
		server := &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 30 * time.Second}
		go func() { failures <- server.Serve(peer) }()
		defer server.Close()
		go func() {
			failures <- applepeer.Serve(ctx, applepeer.Config{BlobCapture: blobStore.Capture, Socket: cfg.Socket, Secret: secret, Commands: commands, Checkpoints: store.Checkpoints, NextFence: store.NextFence, SourceState: store.SourceState, CommitCapture: func(ctx context.Context, c contracts.Capture) (bool, error) {
				result, err := store.CommitCapture(ctx, c)
				if err == nil && !result.Degraded && !result.Duplicate {
					cursor := store.Head()
					cursor.Sequence = result.Through
					commands.RecordCapture(c, cursor)
				}
				return result.Degraded, err
			}})
		}()
		if cfg.HelperApp != "" {
			go func() { failures <- superviseHelper(ctx, cfg, secretPath) }()
		}
	} else {
		client := &replication.Client{Store: store, Authority: authority, Blobs: blobStore}
		service.LocalStatus = client.Status
		service.Target = &operations.RemoteGateway{Authority: authority, Connected: func() bool { return client.Status().State == "connected" }, Wait: client.Wait}
		go func() { failures <- client.Follow(ctx) }()
	}
	server := &http.Server{Handler: frontdoor.Handler(frontdoor.Deps{Operations: service, Authority: authority, Address: cfg.APIAddress, Blobs: blobStore}, contracts.Operations), ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 30 * time.Second}
	go func() { failures <- server.Serve(api) }()
	defer server.Close()
	b, err := json.Marshal(cfg)
	if err != nil {
		return err
	}
	if err = os.WriteFile(filepath.Join(cfg.Home, "runtime.json"), b, 0600); err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return nil
	case err := <-failures:
		if errors.Is(err, http.ErrServerClosed) {
			return nil
		}
		return err
	}
}
