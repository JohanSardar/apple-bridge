package main

import (
	"bufio"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func main() {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	commands := &applepeer.Commands{}
	store, err := state.Open(filepath.Join(filepath.Dir(os.Args[1]), "gateway-state"))
	if err != nil {
		panic(err)
	}
	defer store.Close()
	if err = store.InitCommands(ctx); err != nil {
		panic(err)
	}
	authority, err := trust.Open(filepath.Join(filepath.Dir(os.Args[1]), "gateway-trust"))
	if err != nil {
		panic(err)
	}
	token, err := authority.CreateToken([]contracts.Grant{"admin"})
	if err != nil {
		panic(err)
	}
	owner, err := authority.AuthenticateToken(token)
	if err != nil {
		panic(err)
	}
	_, agentToken, err := authority.CreateAgent("Synthetic probe", trust.AgentApprove, []contracts.Grant{"reminders:write", "reminders:read", "messages:write", "messages:read"})
	if err != nil {
		panic(err)
	}
	agent, err := authority.AuthenticateToken(agentToken)
	if err != nil {
		panic(err)
	}
	gateway := &operations.LocalGateway{State: store, Authority: authority, Peer: commands, Context: ctx}
	go applepeer.Serve(ctx, applepeer.Config{Socket: os.Args[1], Secret: []byte{7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7}, Commands: commands,
		NextFence:   store.NextFence,
		Checkpoints: store.Checkpoints,
		CommitCapture: func(_ context.Context, c contracts.Capture) (bool, error) {
			result, err := store.CommitCapture(ctx, c)
			if err == nil {
				commands.RecordCapture(c, store.Head())
			}
			return result.Degraded, err
		},
		SourceState: func(context.Context, contracts.Source, string, contracts.Coverage, []string, string) error {
			return nil
		},
	})
	scanner := bufio.NewScanner(os.Stdin)
	encoder := json.NewEncoder(os.Stdout)
	for scanner.Scan() {
		var frame contracts.IPCFrame
		if json.Unmarshal(scanner.Bytes(), &frame) != nil {
			os.Exit(2)
		}
		if frame.T == "Available" {
			encoder.Encode(map[string]bool{"available": commands.Available()})
			continue
		}
		p := owner
		if frame.Principal == "agent:probe" {
			p = agent
		}
		var record contracts.CommandRecord
		var localErr error
		handled := true
		switch frame.T {
		case "Submit":
			var command contracts.Command
			if json.Unmarshal(frame.Args, &command) != nil {
				panic("command")
			}
			record, localErr = gateway.Submit(ctx, p, command)
		case "Status":
			record, localErr = gateway.Status(ctx, p, contracts.CommandQuery{ID: frame.CommandID, Resolve: frame.Code == "resolve"})
		case "Decide":
			record, localErr = store.Command(ctx, frame.CommandID)
			if localErr == nil {
				record, localErr = gateway.DecidePhoneApproval(ctx, frame.CommandID, record.RequestHash, frame.Code == "approve")
			}
		case "Recover":
			localErr = gateway.Recover(ctx)
		default:
			handled = false
		}
		if handled {
			if localErr != nil {
				encoder.Encode(map[string]any{"error": localErr})
			} else {
				encoder.Encode(record)
			}
			continue
		}
		bounded, stop := context.WithTimeout(ctx, 3*time.Second)
		answer, err := commands.Exchange(bounded, frame)
		stop()
		if err != nil {
			encoder.Encode(map[string]any{"error": map[string]string{"code": "unavailable"}})
		} else {
			encoder.Encode(answer)
		}
	}
}
