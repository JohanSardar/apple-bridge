package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func uploadSubmit(ctx context.Context, w *world.World, socket *websocket.Conn, command contracts.Command, lose bool) (contracts.CommandRecord, error) {
	if !lose {
		return uploadCommandReply(ctx, socket, command)
	}
	bounded, stop := context.WithTimeout(ctx, 25*time.Second)
	defer stop()
	if err := wsjson.Write(bounded, socket, map[string]any{"t": "cmd", "id": command.ID, "op": command.Op, "args": command.Args}); err != nil {
		return contracts.CommandRecord{}, err
	}
	err := w.Await(25*time.Second, func() bool {
		effects, err := w.Effects()
		record, readErr := uploadGatewayCommand(bounded, w.Gateway.Home, command.ID)
		return err == nil && effects[command.ID] == 1 && readErr == nil && record.State == contracts.CommandObserved
	})
	socket.CloseNow()
	if err != nil {
		return contracts.CommandRecord{}, err
	}
	return contracts.CommandRecord{Command: command, State: contracts.CommandIndeterminate, Disposition: contracts.DispositionIndeterminate, Error: contracts.ErrIndeterminate}, contracts.ErrIndeterminate
}
func uploadReconcile(ctx context.Context, c *webClient, id string) (contracts.CommandRecord, error) {
	var result contracts.CommandRecord
	socket, err := c.socket()
	if err != nil {
		return result, err
	}
	defer socket.CloseNow()
	bounded, stop := context.WithTimeout(ctx, 25*time.Second)
	defer stop()
	if err = wsjson.Write(bounded, socket, map[string]any{"t": "sub", "id": "command:" + id, "view": "record", "args": contracts.ReadQuery{Collection: "system.command", ID: id}}); err != nil {
		return result, err
	}
	for {
		var frame struct {
			T       string               `json:"t"`
			Data    contracts.ReadResult `json:"data"`
			Changes []contracts.Change   `json:"changes"`
		}
		if err = wsjson.Read(bounded, socket, &frame); err != nil {
			return result, err
		}
		records := frame.Data.Records
		for _, change := range frame.Changes {
			records = append(records, change.Record)
		}
		for _, record := range records {
			if record.ID == id && json.Unmarshal(record.Data, &result) == nil && result.Command.ID == id && result.Disposition != "" {
				return result, nil
			}
		}
	}
}

func uploadCommandReply(ctx context.Context, socket *websocket.Conn, command contracts.Command) (contracts.CommandRecord, error) {
	bounded, stop := context.WithTimeout(ctx, 25*time.Second)
	defer stop()
	unknown := func(err error) (contracts.CommandRecord, error) {
		return contracts.CommandRecord{Command: command, State: contracts.CommandIndeterminate, Disposition: contracts.DispositionIndeterminate, Error: contracts.ErrIndeterminate}, err
	}
	if err := wsjson.Write(bounded, socket, map[string]any{"t": "cmd", "id": command.ID, "op": command.Op, "args": command.Args}); err != nil {
		return unknown(err)
	}
	for {
		var frame struct {
			T       string                  `json:"t"`
			ID      string                  `json:"id"`
			Command contracts.CommandRecord `json:"command"`
			Error   *contracts.Error        `json:"error"`
		}
		if err := wsjson.Read(bounded, socket, &frame); err != nil {
			return unknown(err)
		}
		if frame.T == "ack" && frame.ID == command.ID && frame.Error != nil {
			if frame.Error.Code == "unavailable" || frame.Error.Code == "indeterminate" {
				return unknown(frame.Error)
			}
			return contracts.CommandRecord{Command: command, State: contracts.CommandRejected, Error: frame.Error}, frame.Error
		}
		if frame.T == "cmd_status" && frame.Command.Command.ID == command.ID && frame.Command.Disposition != "" {
			if frame.Error != nil {
				return frame.Command, frame.Error
			}
			return frame.Command, nil
		}
	}
}
