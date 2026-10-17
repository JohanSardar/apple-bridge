package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	bridgeruntime "github.com/JohanSardar/apple-bridge/internal/runtime"
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
	home := set.String("home", filepath.Join(os.Getenv("HOME"), ".local", "share", "apple-bridge"), "private state directory")
	role := set.String("role", "gateway", "gateway or replica")
	peer := set.String("peer-addr", "", "gateway peer listen address")
	api := set.String("api-addr", "", "loopback API address")
	socket := set.String("socket", "", "ApplePeer socket path")
	scope := set.String("scope", "reminders:read,system:read", "comma separated grants")
	set.String("name", "", "replica display name")
	code := ""
	if verb == "join" && len(args) > 0 && args[0][0] != '-' {
		code = args[0]
		args = args[1:]
	}
	if err := set.Parse(args); err != nil {
		return err
	}
	if verb == "start" {
		return bridgeruntime.Start(ctx, bridgeruntime.Config{Role: *role, Home: *home, PeerAddress: *peer, APIAddress: *api, Socket: *socket})
	}
	authority, err := trust.Open(*home)
	if err != nil {
		return err
	}
	switch verb {
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
		code, err := authority.Pair(cfg.PeerAddress, grants)
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
