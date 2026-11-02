package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/agent"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scenarios"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func run(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("expected run or simapple")
	}
	switch args[0] {
	case "inventory":
		return inventory(args[1:])
	case "wire-checks":
		if len(args) != 2 {
			return contracts.ErrInvalid
		}
		count, err := contracts.CheckIPCFixtures(args[1])
		if err == nil {
			fmt.Printf("wire fixtures: %d decoded\n", count)
		}
		return err
	case "credential-cleanup":
		set := flag.NewFlagSet("credential-cleanup", flag.ContinueOnError)
		home := set.String("home", "", "isolated credential directory")
		if err := set.Parse(args[1:]); err != nil {
			return err
		}
		return world.CleanupCredentials(ctx, *home)
	case "credential-lifecycle":
		set := flag.NewFlagSet("credential-lifecycle", flag.ContinueOnError)
		home := set.String("home", "", "isolated credential directory")
		if err := set.Parse(args[1:]); err != nil {
			return err
		}
		result, err := world.CredentialLifecycle(ctx, *home)
		if err != nil {
			return err
		}
		return json.NewEncoder(os.Stdout).Encode(result)
	case "simapple":
		set := flag.NewFlagSet("simapple", flag.ContinueOnError)
		socket := set.String("socket", "", "ApplePeer socket")
		secret := set.String("secret-file", "", "installation secret file")
		control := set.String("control", "", "control Unix socket")
		legacySources := set.Bool("legacy-sources", false, "simulate the legacy helper source catalog")
		if err := set.Parse(args[1:]); err != nil {
			return err
		}
		if *socket == "" || *secret == "" || *control == "" {
			return fmt.Errorf("simapple paths required")
		}
		return simapple.Run(ctx, simapple.Config{Socket: *socket, SecretFile: *secret, Control: *control, LegacySources: *legacySources})
	case "run":
		level := "sim"
		asJSON := false
		paths := []string{}
		for i := 1; i < len(args); i++ {
			switch args[i] {
			case "--level":
				i++
				if i >= len(args) {
					return fmt.Errorf("level required")
				}
				level = args[i]
			case "--prebuilt":
				ctx = world.WithPrebuilt(ctx)
			case "--direct-tailscale":
				ctx = world.WithOptions(ctx, world.Options{DirectTailscale: true})
			case "--require-quiet":
				ctx = scenarios.RequireQuiet(ctx)
			case "--json":
				asJSON = true
			default:
				paths = append(paths, args[i])
			}
		}
		if len(paths) == 0 {
			return fmt.Errorf("scenario paths required")
		}
		if len(paths) == 1 && filepath.Base(paths[0]) == "QUERY-AGENT.yaml" && (level == "sim" || level == "real" || level == "agent") {
			return agent.Query(ctx, level, asJSON)
		}
		if level == "agent" {
			if len(paths) == 1 && filepath.Base(paths[0]) == "MAIL-AGENT.yaml" {
				return agent.Mail(ctx, asJSON)
			}
			return agent.Suite(ctx, paths, asJSON)
		}
		return scenarios.Suite(ctx, paths, level, asJSON)
	}
	return fmt.Errorf("unknown lab verb")
}
func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
