package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket/wsjson"
)

func uploadFileCountCheck(ctx context.Context, c *webClient, add func(string, bool)) {
	socket, err := c.socket()
	add("file_count_live_channel", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	files := make([]contracts.MessageFile, 11)
	for i := range files {
		files[i] = contracts.MessageFile{Name: "Bridge Test.txt", Data: base64.StdEncoding.EncodeToString([]byte("Bridge Test"))}
	}
	args, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "Bridge Test file limit", "files": files})
	id := contracts.NewCommandID()
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	err = wsjson.Write(bounded, socket, map[string]any{"t": "cmd", "id": id, "op": "messages.send", "args": json.RawMessage(args)})
	if err != nil {
		add("eleven_files_rejected_before_admission", false)
		return
	}
	for {
		var frame struct {
			T      string           `json:"t"`
			ID     string           `json:"id"`
			Status string           `json:"status"`
			Error  *contracts.Error `json:"error"`
		}
		if wsjson.Read(bounded, socket, &frame) != nil {
			add("eleven_files_rejected_before_admission", false)
			return
		}
		if frame.ID == id && frame.T == "ack" {
			add("eleven_files_rejected_before_admission", frame.Status == "rejected" && frame.Error != nil && frame.Error.Code == "invalid")
			return
		}
	}
}
