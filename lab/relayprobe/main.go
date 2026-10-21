package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/signal"
	"syscall"
)

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	var err error
	if len(os.Args) < 2 {
		err = fmt.Errorf("probe action required")
	} else {
		switch os.Args[1] {
		case "proxy":
			err = proxy(ctx, os.Args[2:])
		case "replay":
			err = replay(ctx)
		case "query":
			err = query(ctx)
		default:
			err = fmt.Errorf("unknown probe action")
		}
	}
	if err != nil {
		json.NewEncoder(os.Stdout).Encode(map[string]string{"error": err.Error()})
		os.Exit(1)
	}
}
