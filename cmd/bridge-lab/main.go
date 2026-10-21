package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/agent"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scenarios"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func run(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("expected run or simapple")
	}
	switch args[0] {
	case "demo":
		set := flag.NewFlagSet("demo", flag.ContinueOnError)
		listen := set.String("listen", "127.0.0.1:0", "loopback web listener")
		seed := set.Int64("seed", 1, "synthetic dataset seed")
		huge := set.Bool("huge", false, "include a 100000-message performance chat")
		alive := set.Bool("alive", false, "emit synthetic live messages and reactions")
		if err := set.Parse(args[1:]); err != nil {
			return err
		}
		if err := world.Build(ctx, "sim"); err != nil {
			return err
		}
		started := time.Now()
		w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "admin"})
		if err != nil {
			return err
		}
		defer w.Close()
		dataset := demo.Generate(demo.Options{Seed: *seed, Huge: *huge})
		if err = demo.Populate(w, dataset); err != nil {
			return err
		}
		server, err := demo.Serve(w, *listen)
		if err != nil {
			return err
		}
		defer server.Close()
		if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"url": server.URL, "seed": *seed, "counts": dataset.Counts, "digest": dataset.Digest(), "startup_ms": time.Since(started).Milliseconds(), "alive": *alive}); err != nil {
			return err
		}
		if *alive {
			return demo.Alive(ctx, w, 3*time.Second)
		}
		<-ctx.Done()
		return nil
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
		if level == "agent" {
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
