package runtime

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
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
	Listen      string `json:"listen,omitempty"`
	BlobCap     int64  `json:"blob_cap,omitempty"`
	BlobGraceMS int64  `json:"blob_grace_ms,omitempty"`

	Role              string        `json:"role"`
	Home              string        `json:"home"`
	PeerAddress       string        `json:"peer_address"`
	APIAddress        string        `json:"api_address"`
	Socket            string        `json:"socket"`
	HelperApp         string        `json:"helper_app,omitempty"`
	HelperLabCommands bool          `json:"helper_lab_commands,omitempty"`
	RetentionRows     uint64        `json:"retention_rows"`
	RetentionAge      time.Duration `json:"retention_age"`
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
	defer blobStore.Close()
	store.BlobProtection = blobStore.Protect
	store.BlobDelta = blobStore.UpdateProtection
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
				blobStore.Collect(ctx)
			}
		}
	}()

	if err = serveBlobLab(ctx, cfg.Home, store, blobStore); err != nil {
		return err
	}
	service := &operations.Service{State: store}
	service.LocalStatus = func() contracts.ReplicaStatus {
		return contracts.ReplicaStatus{Role: "gateway", State: "ready", Cursor: store.Head(), GatewayHead: store.Head().Sequence, Blobs: blobStore.Progress()}
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
		gateway := &operations.LocalGateway{Uploads: blobStore, State: store, Authority: authority, Peer: commands, Context: ctx, Audit: func(record contracts.CommandRecord) {
			auditMu.Lock()
			defer auditMu.Unlock()
			json.NewEncoder(audit).Encode(map[string]any{"command_id": record.Command.ID, "operation": record.Command.Op, "request_hash": record.RequestHash, "argument_bytes": len(record.Command.Args), "state": record.State, "at": time.Now().UTC()})
		}}
		if err = gateway.Recover(ctx); err != nil {
			return err
		}
		service.Target = gateway
		policy := state.Retention{Rows: cfg.RetentionRows, Age: cfg.RetentionAge}
		if policy.Rows == 0 {
			policy.Rows = 1000000
		}
		if policy.Age == 0 {
			policy.Age = 7 * 24 * time.Hour
		}
		go func() {
			tick := time.NewTicker(time.Second)
			defer tick.Stop()
			for {
				select {
				case <-ctx.Done():
					return
				case <-tick.C:
					store.Prune(ctx, policy)
				}
			}
		}()
		feed := &replication.Server{Store: store, Authority: authority, Blobs: blobStore, Command: gateway.Forward}
		service.LocalStatus = func() contracts.ReplicaStatus {
			return contracts.ReplicaStatus{Role: "gateway", State: "ready", Cursor: store.Head(), GatewayHead: store.Head().Sequence, Blobs: blobStore.Progress(), DialReplicas: feed.DialStatus()}
		}
		go authority.DialPairings(ctx)
		go feed.Dial(ctx)
		mux := http.NewServeMux()
		mux.Handle("/v1/pair", authority.PairHandler())
		mux.Handle("/v1/feed", feed.Handler())
		mux.Handle("/v1/commands", gateway.Handler())
		server := &http.Server{Handler: mux, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 30 * time.Second}
		go func() { failures <- server.Serve(peer) }()
		defer server.Close()
		go func() {
			failures <- applepeer.Serve(ctx, applepeer.Config{BlobCapture: blobStore.Capture, Socket: cfg.Socket, Secret: secret, Commands: commands, Checkpoints: func(ctx context.Context) (map[contracts.Source]contracts.Checkpoint, error) {
				checkpoints, err := store.Checkpoints(ctx)
				return blobStore.ResumeCheckpoints(checkpoints), err
			}, NextFence: store.NextFence, SourceState: func(ctx context.Context, source contracts.Source, status string, coverage contracts.Coverage, capabilities []string, reason string) error {
				err := store.SourceState(ctx, source, status, coverage, capabilities, reason)
				if state.IsCapacity(err) {
					store.Capacity(source, err)
				}
				return err
			}, CommitCapture: func(ctx context.Context, c contracts.Capture) (bool, error) {
				commitStartedAt := time.Now().UnixNano()
				result, err := store.CommitCapture(ctx, c)
				store.Capacity(c.Source, err)
				if err != nil && cfg.HelperLabCommands {
					code := "capture_error"
					var problem *contracts.Error
					if errors.As(err, &problem) {
						code = problem.Code
					}
					var sqlite interface{ Code() int }
					if errors.As(err, &sqlite) {
						code = fmt.Sprintf("sqlite_%d", sqlite.Code())
					}
					data, _ := json.Marshal(map[string]any{"stage": "gateway_capture_rejected", "source": c.Source, "error_code": code, "error_type": fmt.Sprintf("%T", err), "observations": len(c.Observations)})
					if file, openErr := os.OpenFile(filepath.Join(cfg.Home, "capture-trace.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600); openErr == nil {
						file.Write(append(data, '\n'))
						file.Close()
					}
				}
				committedAt := result.CommittedAt.UnixNano()
				if err == nil && cfg.HelperLabCommands && !result.CommittedAt.IsZero() {
					keys := []string{}
					for _, observation := range c.Observations {
						keys = append(keys, fmt.Sprintf("%x", sha256.Sum256([]byte(observation.NativeKey))))
					}
					data, _ := json.Marshal(map[string]any{"stage": "gateway_commit", "wall_ns": committedAt, "commit_started_wall_ns": commitStartedAt, "checkpoint": c.Checkpoint, "keys": keys})
					if file, openErr := os.OpenFile(filepath.Join(cfg.Home, "capture-trace.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600); openErr == nil {
						file.Write(append(data, '\n'))
						file.Close()
					}
				}
				if err == nil && !result.Degraded && !result.Duplicate {
					cursor := store.Head()
					cursor.Sequence = result.Through
					commands.RecordCapture(c, cursor)
				}
				if err == nil {
					captureBoundary(cfg.Home)
				}
				return result.Degraded, err
			}})
		}()
		if cfg.HelperApp != "" {
			go func() { failures <- superviseHelper(ctx, cfg, secretPath) }()
		}
	} else {
		os.Remove(filepath.Join(cfg.Home, "gateway-clock.json"))
		client := &replication.Client{Store: store, Authority: authority, Blobs: blobStore, GatewayClock: func(clock contracts.ClockSample) {
			sample, _ := json.Marshal(clock)
			path := filepath.Join(cfg.Home, "gateway-clock.json")
			if os.WriteFile(path+".tmp", sample, 0600) == nil {
				os.Rename(path+".tmp", path)
			}
		}}
		service.LocalStatus = client.Status
		service.Target = &operations.RemoteGateway{Authority: authority, Exchange: client.Exchange, Connected: func() bool { return client.Status().State == "connected" }, Wait: client.Wait}
		enrollment, err := authority.Enrollment()
		if err != nil || enrollment == nil {
			return contracts.ErrUnauthorized
		}
		if enrollment.Dial != "" {
			address := cfg.Listen
			if address == "" {
				address = enrollment.Dial
			}
			listener, err := authority.PinnedListener(address, enrollment.Fingerprint)
			if err != nil {
				return err
			}
			defer listener.Close()
			cfg.Listen = listener.Addr().String()
			mux := http.NewServeMux()
			mux.Handle("/v1/feed", client.Handler())
			peerServer := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
			defer peerServer.Close()
			go func() { failures <- peerServer.Serve(listener) }()
		} else {
			if cfg.Listen != "" {
				return contracts.ErrInvalid
			}
			go func() { failures <- client.Follow(ctx) }()
		}
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
