//go:build lab

package main

import (
	"context"
	"encoding/json"
	"flag"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
)

func init() {
	labCommands["lab-session"] = runLabSession
	labCommands["lab-principal"] = runLabPrincipal
}

func labPairURL() string { return os.Getenv("APPLE_BRIDGE_LAB_PAIR_URL") }

func labAuthority(name string, args []string, extra func(*flag.FlagSet)) (*trust.Authority, *flag.FlagSet, error) {
	set := flag.NewFlagSet(name, flag.ContinueOnError)
	home := set.String("home", "", "isolated gateway home")
	if extra != nil {
		extra(set)
	}
	if err := set.Parse(args); err != nil {
		return nil, nil, err
	}
	if *home == "" || set.NArg() != 0 {
		return nil, nil, contracts.ErrInvalid
	}
	authority, err := trust.Open(*home)
	return authority, set, err
}

func runLabSession(_ context.Context, args []string) error {
	authority, _, err := labAuthority("lab-session", args, nil)
	if err != nil {
		return err
	}
	session, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]string{"name": bridgeweb.LocalSessionCookie, "value": session})
}

func runLabPrincipal(_ context.Context, args []string) error {
	var id, fingerprint *string
	authority, _, err := labAuthority("lab-principal", args, func(set *flag.FlagSet) {
		id = set.String("id", "", "principal id")
		fingerprint = set.String("fingerprint", "", "certificate fingerprint")
	})
	if err != nil {
		return err
	}
	var principal trust.Principal
	switch {
	case *id != "" && *fingerprint == "":
		principal, err = authority.Principal(*id)
	case *fingerprint != "" && *id == "":
		principal, err = authority.LabPrincipalByFingerprint(*fingerprint)
	default:
		return contracts.ErrInvalid
	}
	if err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"id": principal.ID, "kind": principal.Kind, "fingerprint": principal.Fingerprint, "grants": principal.Grants, "revision": principal.Revision, "relay2": principal.Relay2})
}
