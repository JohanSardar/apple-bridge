package simapple

import (
	"encoding/json"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	applepeer "github.com/JohanSardar/apple-bridge/lab/ipc"
)

type staleRequest struct {
	Key        string          `json:"key"`
	Collection string          `json:"collection"`
	Data       json.RawMessage `json:"data"`
}

type staleResult struct {
	StaleFence   uint64          `json:"stale_fence"`
	CurrentFence uint64          `json:"current_fence"`
	Reply        string          `json:"reply"`
	Key          string          `json:"key"`
	Collection   string          `json:"collection"`
	Data         json.RawMessage `json:"data"`
}

func staleConnect(socket string, secret []byte) (*applepeer.Wire, contracts.IPCFrame, error) {
	conn, err := net.DialUnix("unix", nil, &net.UnixAddr{Name: socket, Net: "unix"})
	if err != nil {
		return nil, contracts.IPCFrame{}, err
	}
	wire := &applepeer.Wire{Conn: conn}
	if err = applepeer.Authenticate(wire, secret); err != nil {
		conn.Close()
		return nil, contracts.IPCFrame{}, err
	}
	open, _, err := wire.Receive()
	if err != nil {
		conn.Close()
		return nil, open, err
	}
	if open.T != "Open" || open.OwnerFence == 0 {
		conn.Close()
		return nil, open, contracts.ErrInvalid
	}
	ready := contracts.IPCFrame{T: "Ready", Protocol: contracts.IPCProtocol, HelperBuild: "sim/1", OSBuild: "sim"}
	if _, err = wire.Send(ready); err != nil {
		conn.Close()
		return nil, open, err
	}
	return wire, open, nil
}

func staleReply(wire *applepeer.Wire) string {
	deadline := time.Now().Add(3 * time.Second)
	for {
		wire.Conn.SetReadDeadline(deadline)
		var frame contracts.IPCFrame
		_, err := contracts.ReadFrame(wire.Conn, &frame)
		if err != nil {
			if netErr, ok := err.(net.Error); ok && netErr.Timeout() {
				return "silent"
			}
			return "closed"
		}
		switch frame.T {
		case "Committed":
			return "committed"
		case "Ping", "Pong":
		default:
			return "rejected"
		}
	}
}

func (f *fakeApple) staleRoute(mux *http.ServeMux, cfg Config) {
	mux.HandleFunc("POST /stale-frame", func(w http.ResponseWriter, r *http.Request) {
		var request staleRequest
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, contracts.MaxFrame)).Decode(&request) != nil || request.Key == "" || len(request.Data) == 0 {
			http.Error(w, "invalid", 400)
			return
		}
		if request.Collection == "" {
			request.Collection = "reminders.reminder"
		}
		secret, err := os.ReadFile(cfg.SecretFile)
		if err != nil {
			http.Error(w, "secret", 500)
			return
		}
		superseded, supersededOpen, err := staleConnect(cfg.Socket, secret)
		if err != nil {
			http.Error(w, "connect superseded: "+err.Error(), 502)
			return
		}
		defer superseded.Conn.Close()
		current, currentOpen, err := staleConnect(cfg.Socket, secret)
		if err != nil {
			http.Error(w, "connect current: "+err.Error(), 502)
			return
		}
		defer current.Conn.Close()
		source := contracts.CollectionSource(request.Collection)
		offered := currentOpen.Checkpoints[source]
		f.mu.Lock()
		generation := f.generation
		f.mu.Unlock()
		if offered.Generation != "" {
			generation = offered.Generation
		}
		capture := contracts.Capture{Source: source, Generation: generation, Predecessor: offered.Checkpoint, Checkpoint: contracts.NewID("cp"), Observations: []contracts.Observation{{Kind: "upsert", Collection: request.Collection, NativeKey: request.Key, ObservedAt: time.Now().UTC(), Data: request.Data}}}
		reply := "closed"
		if _, err = superseded.Send(contracts.IPCFrame{T: "Capture", Capture: &capture}); err == nil {
			reply = staleReply(superseded)
		}
		json.NewEncoder(w).Encode(staleResult{StaleFence: supersededOpen.OwnerFence, CurrentFence: currentOpen.OwnerFence, Reply: reply, Key: request.Key, Collection: request.Collection, Data: request.Data})
	})
}
