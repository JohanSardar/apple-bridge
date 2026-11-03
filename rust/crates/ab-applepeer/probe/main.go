package main

import (
	"bufio"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"sync"
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
	var cap int64
	if value := os.Getenv("AB_PROBE_BLOB_CAP"); value != "" {
		cap, err = strconv.ParseInt(value, 10, 64)
		if err != nil {
			panic(err)
		}
	}
	blobStore, err := blobs.Open(filepath.Join(filepath.Dir(os.Args[1]), "gateway-blobs"), blobs.Config{Cap: cap})
	if err != nil {
		panic(err)
	}
	var auditMu sync.Mutex
	var audits []operations.QueryAudit
	gateway := &operations.LocalGateway{Uploads: blobStore, QueryAudit: func(a operations.QueryAudit) { auditMu.Lock(); defer auditMu.Unlock(); audits = append(audits, a) }, State: store, Authority: authority, Peer: commands, Context: ctx}
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
	var savedBlob io.Reader
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 16<<20)
	encoder := json.NewEncoder(os.Stdout)
	for scanner.Scan() {
		var frame contracts.IPCFrame
		if json.Unmarshal(scanner.Bytes(), &frame) != nil {
			os.Exit(2)
		}
		if frame.T == "QueryAudits" {
			auditMu.Lock()
			encoder.Encode(audits)
			auditMu.Unlock()
			continue
		}
		if frame.T == "StartBlob" {
			savedBlob, err = commands.QueryBlob(ctx, contracts.QueryBlob{Handle: frame.Handle, Size: frame.Length})
			if err != nil {
				panic(err)
			}
			encoder.Encode(map[string]bool{"started": true})
			continue
		}
		if frame.T == "FinishBlob" {
			_, err := io.ReadAll(savedBlob)
			if err != nil {
				encoder.Encode(map[string]any{"error": map[string]string{"code": "unavailable"}})
			} else {
				encoder.Encode(map[string]bool{"read": true})
			}
			continue
		}
		if frame.T == "DisableUploads" {
			gateway.Uploads = nil
			encoder.Encode(map[string]bool{"disabled": true})
			continue
		}
		if frame.T == "Available" {
			encoder.Encode(map[string]bool{"available": commands.Available()})
			continue
		}
		if frame.T == "ReadBlob" {
			reader, err := commands.QueryBlob(ctx, contracts.QueryBlob{Handle: frame.Handle, Size: frame.Offset + frame.Length})
			var data []byte
			if err == nil {
				data, err = io.ReadAll(reader)
			}
			if err != nil {
				encoder.Encode(map[string]any{"error": err})
			} else {
				encoder.Encode(map[string]any{"bytes": data, "eof": true})
			}
			continue
		}
		p := owner
		if frame.Principal == "agent:probe" {
			p = agent
		}
		var record contracts.CommandRecord
		var localErr error
		handled := true
		if frame.T == "LocalQuery" {
			var query contracts.Query
			if json.Unmarshal(frame.Args, &query) != nil {
				panic("query")
			}
			result, err := gateway.Query(ctx, p, query)
			if err != nil {
				encoder.Encode(map[string]any{"error": err})
			} else {
				encoder.Encode(result)
			}
			continue
		}
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
