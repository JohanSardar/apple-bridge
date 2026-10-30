package iphone

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
	"time"
)

func IsLiveCommand(op string) bool {
	return op == "iphone.live.start" || op == "iphone.live.update" || op == "iphone.live.end"
}

func LivePayload(p trust.Principal, command contracts.Command, now time.Time) (string, json.RawMessage, error) {
	var args contracts.IPhoneLiveArgs
	if !IsLiveCommand(command.Op) || contracts.ValidateIPhoneCommand(command) != nil || json.Unmarshal(command.Args, &args) != nil {
		return "", nil, contracts.ErrInvalid
	}
	event := strings.TrimPrefix(command.Op, "iphone.live.")
	key := args.TaskID
	if event == "start" {
		key = "_start"
	}
	token := p.IPhoneLiveTokens[key]
	if token == "" {
		return "", nil, contracts.ErrUnavailable
	}
	aps := map[string]any{"timestamp": now.Unix(), "event": event, "content-state": map[string]any{"status": args.Status, "progress": args.Progress}}
	if event == "start" {
		aps["attributes-type"] = "AgentTaskAttributes"
		aps["attributes"] = map[string]string{"taskID": args.TaskID, "title": args.Title}
		aps["alert"] = map[string]string{"title": "Apple Bridge", "body": args.Title}
	}
	if event == "end" {
		aps["dismissal-date"] = now.Unix()
	}
	raw, err := json.Marshal(aps)
	return token, raw, err
}
