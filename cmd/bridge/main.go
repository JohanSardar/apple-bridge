package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"syscall"
	"time"

	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
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
	if verb == "token" {
		if len(args) == 0 || args[0] != "create" {
			return contracts.ErrInvalid
		}
		args = args[1:]
	}
	set := flag.NewFlagSet(verb, flag.ContinueOnError)
	home := set.String("home", defaultHome(), "private state directory")
	role := set.String("role", "gateway", "gateway or replica")
	peer := set.String("peer-addr", "", "gateway peer listen address")
	api := set.String("api-addr", "", "loopback API address")
	helperApp := set.String("helper-app", "", "signed Helper app for real mode")
	helperLabCommands := set.Bool("helper-lab-commands", false, "enable unproven native commands in isolated lab worlds")
	socket := set.String("socket", "", "ApplePeer socket path")
	blobCap := set.Int64("blob-cap", 0, "machine blob cap in bytes")
	blobGrace := set.Int64("blob-grace-ms", 0, "unreferenced blob grace in milliseconds")
	scope := set.String("scope", "reminders:read,system:read", "comma separated grants")
	set.String("name", "", "replica display name")
	backup := set.String("backup", "", "state backup path")
	acceptLedgerLoss := set.Bool("accept-ledger-loss", false, "acknowledge lost command history")
	pairTTL := set.Duration("expires-in", 10*time.Minute, "pairing code lifetime")
	retentionRows := set.Uint64("retention-rows", 0, "minimum retained journal rows")
	retentionAge := set.Duration("retention-age", 0, "minimum journal age")
	code := ""
	if (verb == "join" || verb == "restore" || verb == "backup" || verb == "revoke" || verb == "scope") && len(args) > 0 && len(args[0]) > 0 && args[0][0] != '-' {
		code = args[0]
		args = args[1:]
	}
	if err := set.Parse(args); err != nil {
		return err
	}
	if verb == "start" {
		return bridgeruntime.Start(ctx, bridgeruntime.Config{Role: *role, Home: *home, PeerAddress: *peer, APIAddress: *api, Socket: *socket, HelperApp: *helperApp, HelperLabCommands: *helperLabCommands, BlobCap: *blobCap, BlobGraceMS: *blobGrace, RetentionRows: *retentionRows, RetentionAge: *retentionAge})
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
		code, err := authority.PairTTL(address, grants, *pairTTL)
		if err != nil {
			return err
		}
		fmt.Println(code)
	case "join":
		if code == "" && set.NArg() > 0 {
			code = set.Arg(0)
		}
		return authority.Join(ctx, code)
	case "token":
		grants, err := trust.ParseGrants(*scope)
		if err != nil {
			return err
		}
		token, err := authority.CreateToken(grants)
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
