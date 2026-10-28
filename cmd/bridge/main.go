package main

import (
	"bytes"
	"context"
	"flag"
	"fmt"
	"net/url"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"tailscale.com/util/qrcodes"
	"time"

	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	bridgeruntime "github.com/JohanSardar/apple-bridge/internal/runtime"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func run(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return contracts.ErrInvalid
	}
	verb := args[0]
	args = args[1:]
	if verb == "selftest" {
		if len(args) == 0 || args[0] != "messages" {
			return contracts.ErrInvalid
		}
		args = args[1:]
	}
	if verb == "token" {
		if len(args) == 0 || args[0] != "create" {
			return contracts.ErrInvalid
		}
		args = args[1:]
	}
	agentAction := ""
	if verb == "agent" {
		if len(args) == 0 {
			return contracts.ErrInvalid
		}
		agentAction = args[0]
		args = args[1:]
	}
	webAction := ""
	if verb == "web" || verb == "remote" {
		if len(args) == 0 {
			return contracts.ErrInvalid
		}
		webAction = args[0]
		args = args[1:]
	}
	set := flag.NewFlagSet(verb, flag.ContinueOnError)
	noSelfTestUI := set.Bool("no-ui", false, "defer Messages UI steps and retain cleanup obligations")
	forceSelfTest := set.Bool("force", false, "rerun the Messages self-test on this OS build")
	via := set.String("via", "", "remote backend: embedded (default) or tailscale-app")
	domain := set.String("domain", "", "custom public hostname for relay")
	publicEnroll := set.Bool("public", false, "enroll a passkey at the active public origin")
	snippets := set.Bool("snippets", false, "print remote connection snippets")
	remoteToken := set.Bool("remote", false, "allow this token through Remote")
	home := set.String("home", defaultHome(), "private state directory")
	webHost := set.String("host", "http://localhost:7780", "web serving origin")
	role := set.String("role", "gateway", "gateway or replica")
	listen := set.String("listen", "", "replica listen address")
	dialReplica := set.String("dial-replica", "", "gateway dials replica address")
	viaRemote := set.Bool("via-remote", false, "pair through the public Remote URL")
	peer := set.String("peer-addr", "", "gateway peer listen address")
	api := set.String("api-addr", "", "loopback API address")
	helperApp := set.String("helper-app", "", "signed Helper app for real mode")
	helperLabCommands := set.Bool("helper-lab-commands", false, "enable unproven native commands in isolated lab worlds")
	socket := set.String("socket", "", "ApplePeer socket path")
	blobCap := set.Int64("blob-cap", 0, "machine blob cap in bytes")
	blobGrace := set.Int64("blob-grace-ms", 0, "unreferenced blob grace in milliseconds")
	scope := set.String("scope", "reminders:read,system:read", "comma separated grants")
	name := set.String("name", "", "display name")
	level := set.String("level", "approve", "agent access level: read, approve, full")
	backup := set.String("backup", "", "state backup path")
	acceptLedgerLoss := set.Bool("accept-ledger-loss", false, "acknowledge lost command history")
	pairTTL := set.Duration("expires-in", 10*time.Minute, "pairing code lifetime")
	retentionRows := set.Uint64("retention-rows", 0, "minimum retained journal rows")
	retentionAge := set.Duration("retention-age", 0, "minimum journal age")
	code := ""
	if verb == "agent" && agentAction == "revoke" && len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		code = args[0]
		args = args[1:]
	}
	if (verb == "join" || verb == "restore" || verb == "backup" || verb == "revoke" || verb == "scope") && len(args) > 0 && len(args[0]) > 0 && args[0][0] != '-' {
		code = args[0]
		args = args[1:]
	}
	if err := set.Parse(args); err != nil {
		return err
	}
	if *helperApp == "" && (verb == "selftest" || verb == "setup") {
		executable, _ := os.Executable()
		bundle := filepath.Dir(filepath.Dir(filepath.Dir(executable)))
		if _, err := os.Stat(filepath.Join(bundle, "Contents", "Library", "LaunchAgents", "dev.applebridge.gateway.plist")); err == nil {
			*helperApp = bundle
		}
	}
	if verb == "selftest" || verb == "setup" {
		data, err := bridgeruntime.MessagesSelfTest(ctx, *helperApp, *forceSelfTest, !*noSelfTestUI)
		if len(data) > 0 {
			fmt.Fprintln(os.Stdout, string(data))
		}
		return err
	}
	if verb == "remote" {
		if *snippets {
			if webAction != "status" {
				return contracts.ErrInvalid
			}
			return remote.Snippets(*home, os.Stdout)
		}
		return remote.CommandDomain(ctx, *home, webAction, *via, *name, *domain, os.Stdout)
	}
	if verb == "install-credentials" {
		_, err := trust.Open(*home)
		return err
	}
	if verb == "start" {
		if *helperApp == "" {
			executable, _ := os.Executable()
			bundle := filepath.Dir(filepath.Dir(filepath.Dir(executable)))
			if _, err := os.Stat(filepath.Join(bundle, "Contents", "Library", "LaunchAgents", "dev.applebridge.gateway.plist")); err == nil {
				*helperApp = bundle
			}
		}
		return bridgeruntime.Start(ctx, bridgeruntime.Config{Role: *role, Home: *home, PeerAddress: *peer, Listen: *listen, APIAddress: *api, Socket: *socket, HelperApp: *helperApp, HelperLabCommands: *helperLabCommands, BlobCap: *blobCap, BlobGraceMS: *blobGrace, RetentionRows: *retentionRows, RetentionAge: *retentionAge})
	}
	if verb == "doctor" {
		result, err := bridgeruntime.Doctor(ctx, *home)
		if err != nil {
			return err
		}
		return json.NewEncoder(os.Stdout).Encode(result)
	}
	if verb == "restore" {
		if *backup == "" {
			*backup = code
		}
		return bridgeruntime.Restore(ctx, *home, *backup, *acceptLedgerLoss)
	}
	if verb == "backup" {
		store, err := state.InspectHome(*home)
		if err != nil {
			return err
		}
		defer store.Close()
		if code == "" {
			return contracts.ErrInvalid
		}
		return store.Backup(ctx, code)
	}
	authority, err := trust.Open(*home)
	if err != nil {
		return err
	}
	switch verb {
	case "web":
		switch webAction {
		case "enroll":
			if *publicEnroll {
				var output bytes.Buffer
				if err := remote.Command(ctx, *home, "status", "", "", &output); err != nil {
					return err
				}
				var status remote.Status
				if json.Unmarshal(output.Bytes(), &status) != nil || status.State != "on" || !strings.HasPrefix(status.URL, "https://") {
					return fmt.Errorf("public Remote must be on before enrollment")
				}
				*webHost = status.URL
			}
			origin, err := url.Parse(*webHost)
			if err != nil || origin.Host == "" || origin.Path != "" || origin.RawQuery != "" || origin.Fragment != "" || origin.User != nil || origin.Scheme != "https" && !(origin.Scheme == "http" && origin.Hostname() == "localhost") {
				return contracts.ErrInvalid
			}
			token, err := authority.CreateWebEnroll()
			if err != nil {
				return err
			}
			enrollmentURL := strings.TrimSuffix(*webHost, "/") + "/enroll#" + token
			fmt.Println(enrollmentURL)
			if *publicEnroll {
				_, err = qrcodes.Fprintln(os.Stdout, qrcodes.FormatSmall, enrollmentURL)
				return err
			}
			return nil
		case "lock":
			return authority.LockWeb()
		case "devices":
			devices, err := authority.DeviceSessions()
			if err != nil {
				return err
			}
			return json.NewEncoder(os.Stdout).Encode(devices)
		default:
			return contracts.ErrInvalid
		}
	case "agent":
		switch agentAction {
		case "create":
			grants, err := trust.ParseGrants(*scope)
			if err != nil {
				return err
			}
			agent, token, err := authority.CreateAgent(*name, trust.AgentLevel(*level), grants)
			if err != nil {
				return err
			}
			return json.NewEncoder(os.Stdout).Encode(map[string]any{"agent": agent, "token": token})
		case "list":
			agents, err := authority.Agents()
			if err != nil {
				return err
			}
			return json.NewEncoder(os.Stdout).Encode(agents)
		case "revoke":
			id := code
			if id == "" && set.NArg() > 0 {
				id = set.Arg(0)
			}
			if !strings.HasPrefix(id, "agent:") {
				return contracts.ErrInvalid
			}
			return authority.Revoke(id)
		default:
			return contracts.ErrInvalid
		}
	case "revoke":
		return authority.Revoke(code)
	case "scope":
		grants, err := trust.ParseGrants(*scope)
		if err != nil {
			return err
		}
		return authority.ChangeScope(code, grants)
	case "pair":
		cfg, err := bridgeruntime.LoadConfig(*home)
		if err != nil {
			return err
		}
		if cfg.Role != "gateway" {
			return contracts.ErrUnsupported
		}
		grants, err := trust.ParseGrants(*scope)
		if err != nil {
			return err
		}
		address := cfg.PeerAddress
		if *peer != "" {
			address = *peer
		}
		var code string
		if *viaRemote {
			code, err = remotePairCode(ctx, authority, *home, *peer, *dialReplica, grants, *pairTTL)
		} else {
			code, err = authority.PairDial(address, *dialReplica, grants, *pairTTL)
		}
		if err != nil {
			return err
		}
		fmt.Println(code)
	case "join":
		if code == "" && set.NArg() > 0 {
			code = set.Arg(0)
		}
		if *listen != "" {
			return authority.JoinListen(ctx, code, *listen)
		}
		return authority.Join(ctx, code)
	case "token":
		grants, err := trust.ParseGrants(*scope)
		if err != nil {
			return err
		}
		token, err := authority.CreateAPIToken(grants, *remoteToken)
		if err != nil {
			return err
		}
		fmt.Println(token)
	default:
		return contracts.ErrUnsupported
	}
	return nil
}
func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func defaultHome() string {
	if runtime.GOOS == "darwin" {
		return filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridge")
	}
	root := os.Getenv("XDG_DATA_HOME")
	if root == "" {
		root = filepath.Join(os.Getenv("HOME"), ".local", "share")
	}
	return filepath.Join(root, "apple-bridge")
}
