package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
)

func main() {
	ctx := context.Background()
	if len(os.Args) > 2 {
		if err := state.Restore(ctx, os.Args[1], os.Args[2]); err != nil {
			panic(err)
		}
		return
	}
	s, err := state.Open(os.Args[1])
	if err != nil {
		panic(err)
	}
	defer s.Close()
	if err = s.InitCommands(ctx); err != nil {
		panic(err)
	}
	frozen := map[string]*state.FrozenSnapshot{}
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 16<<20)
	for scanner.Scan() {
		var q struct {
			Op         string                  `json:"op"`
			Capture    contracts.Capture       `json:"capture"`
			From       contracts.CommandState  `json:"from"`
			Record     contracts.CommandRecord `json:"record"`
			ID         string                  `json:"id"`
			Visibility trust.Visibility        `json:"visibility"`
			Revision   uint64                  `json:"revision"`
			Rows       uint64                  `json:"rows"`
			Path       string                  `json:"path"`
			Collection string                  `json:"collection"`
			Query      contracts.ReadQuery     `json:"query"`
		}
		if err = json.Unmarshal(scanner.Bytes(), &q); err != nil {
			panic(err)
		}
		var out any
		err = nil
		switch q.Op {
		case "capture":
			out, err = s.CommitCapture(ctx, q.Capture)
		case "head":
			out = s.Head()
		case "era":
			out, err = s.AdmissionEra(ctx)
		case "fence":
			out, err = s.NextFence(ctx)
		case "checkpoints":
			out, err = s.Checkpoints(ctx)
		case "command":
			out, err = s.Command(ctx, q.ID)
		case "commands":
			out, err = s.Commands(ctx)
		case "transition":
			out, err = s.TransitionCommand(ctx, state.CommandTransition{From: q.From, Record: q.Record})
		case "parts":
			out, err = s.UpdateCommandParts(ctx, q.Record)
		case "freeze":
			var f *state.FrozenSnapshot
			f, err = s.FreezeSnapshot(ctx, q.Visibility, q.Revision)
			if err == nil {
				frozen[f.ID] = f
				out = map[string]any{"id": f.ID, "cursor": f.Cursor, "count": f.Count, "digest": f.Digest, "expires_at": f.ExpiresAt, "records": f.Records}
			}
		case "renew":
			err = frozen[q.ID].Renew(ctx)
			out = map[string]any{}
		case "close":
			err = frozen[q.ID].Close()
			out = map[string]any{}
		case "prune":
			err = s.Prune(ctx, state.Retention{Rows: q.Rows})
			out = map[string]any{}
		case "backup":
			err = s.Backup(ctx, q.Path)
			out = map[string]any{}
		case "native":
			out, err = s.NativeKey(ctx, q.Collection, q.ID)
		case "read":
			out, err = s.Read(ctx, q.Visibility, q.Query)
		}
		if err != nil {
			out = map[string]any{"error": err.Error()}
		}
		raw, _ := json.Marshal(out)
		fmt.Println(string(raw))
	}
	if scanner.Err() != nil {
		panic(scanner.Err())
	}
}
