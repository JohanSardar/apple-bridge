package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/signal"
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
		if err := set.Parse(args[1:]); err != nil {
			return err
		}
		if *socket == "" || *secret == "" || *control == "" {
			return fmt.Errorf("simapple paths required")
		}
		return simapple.Run(ctx, simapple.Config{Socket: *socket, SecretFile: *secret, Control: *control})
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
			case "--direct-tailscale":
				ctx = world.WithOptions(ctx, world.Options{DirectTailscale: true})
			case "--json":
				asJSON = true
			default:
				paths = append(paths, args[i])
			}
		}
		if len(paths) == 0 {
			return fmt.Errorf("scenario paths required")
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
