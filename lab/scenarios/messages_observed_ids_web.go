package scenarios

import (
	"context"
	"net/url"
	"os/exec"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
)

func checkWebCommandIDs(ctx context.Context, w *world.World, chatID string, add func(string, bool)) {
	bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	_, port, _ := strings.Cut(w.Gateway.Host, ":")
	origin := "http://localhost:" + port
	client := webClient{ctx: bounded, origin: origin}
	output, err := exec.CommandContext(bounded, "bin/bridge", "web", "enroll", "--home", w.Gateway.Home, "--host", origin).Output()
	if err != nil {
		add("web_enrollment", false)
		return
	}
	link, err := url.Parse(strings.TrimSpace(string(output)))
	if err != nil {
		add("web_enrollment_url", false)
		return
	}
	authenticator, err := newWebAuthenticator()
	if err != nil {
		add("web_authenticator", false)
		return
	}
	status, _, err := client.signIn(authenticator, "enroll", link.Fragment, "trusted")
	if err != nil || status != 200 {
		add("web_session", false)
		return
	}
	conn, err := client.socket()
	if err != nil {
		add("web_socket", false)
		return
	}
	defer conn.CloseNow()
	id := contracts.NewCommandID()
	err = wsjson.Write(bounded, conn, map[string]any{"t": "cmd", "id": id, "op": "messages.send", "args": map[string]any{"chat_id": chatID, "text": "[ab-test web-observed-ids] message"}})
	add("web_command_submitted", err == nil)
	for err == nil {
		var frame struct {
			Type    string `json:"t"`
			Command struct {
				IDs         []string `json:"observed_record_ids"`
				CommitSeq   uint64   `json:"commit_seq"`
				Disposition string   `json:"disposition"`
				Evidence    struct {
					NativeKey string `json:"native_key"`
				} `json:"evidence"`
			} `json:"command"`
		}
		err = wsjson.Read(bounded, conn, &frame)
		if err != nil {
			break
		}
		if frame.Type != "cmd_status" || frame.Command.Disposition == "" {
			continue
		}
		aliases, problem := gatewayAliases(w)
		expected := aliases[aliasKey(contracts.Messages, "messages.message", frame.Command.Evidence.NativeKey)]
		add("web_cmd_status_exact_observed_id", problem == nil && expected != "" && reflect.DeepEqual(frame.Command.IDs, []string{expected}) && frame.Command.CommitSeq > 0)
		conn.CloseNow()
		reconnected, problem := client.socket()
		if problem != nil {
			add("web_status_reconnect", false)
			return
		}
		defer reconnected.CloseNow()
		problem = wsjson.Write(bounded, reconnected, map[string]any{"t": "sub", "id": "recover-command", "view": "record", "args": map[string]any{"collection": "system.command", "id": id}})
		var snapshot struct {
			Type string `json:"t"`
			Data struct {
				Records []struct {
					Data struct {
						IDs       []string `json:"observed_record_ids"`
						CommitSeq uint64   `json:"commit_seq"`
					} `json:"data"`
				} `json:"records"`
			} `json:"data"`
		}
		if problem == nil {
			problem = wsjson.Read(bounded, reconnected, &snapshot)
		}
		add("web_reconnect_record_retains_exact_ids", problem == nil && snapshot.Type == "snap" && len(snapshot.Data.Records) == 1 && reflect.DeepEqual(snapshot.Data.Records[0].Data.IDs, []string{expected}) && snapshot.Data.Records[0].Data.CommitSeq == frame.Command.CommitSeq)
		return
	}
	add("web_command_observation", false)
}
