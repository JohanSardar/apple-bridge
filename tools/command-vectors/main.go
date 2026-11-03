package main

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"time"
)

type input struct {
	Command contracts.Command `json:"command"`
	Context context           `json:"context"`
}
type context struct {
	Origin          string                        `json:"origin"`
	NativeKey       string                        `json:"native_key"`
	ListKey         string                        `json:"list_key"`
	ExpectedListKey string                        `json:"expected_list_key"`
	FolderKey       string                        `json:"folder_key"`
	ResendKey       string                        `json:"resend_key"`
	Files           []contracts.NativeMessageFile `json:"files"`
}

func main() {
	if len(os.Args) > 1 {
		policies()
		return
	}
	var inputs []input
	if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil {
		panic(err)
	}
	out := []map[string]any{}
	for _, in := range inputs {
		c := in.Command
		a, err := contracts.ValidateCommand(c)
		row := map[string]any{"command": c, "context": in.Context, "accepted": err == nil, "code": "", "message": ""}
		if err != nil {
			e, ok := err.(*contracts.Error)
			if !ok {
				panic(err)
			}
			row["code"] = e.Code
			row["message"] = e.Message
		} else {
			action := contracts.NativeAction{Op: c.Op, Args: a, Origin: in.Context.Origin, NativeKey: in.Context.NativeKey, ListKey: in.Context.ListKey, ExpectedListKey: in.Context.ExpectedListKey, FolderKey: in.Context.FolderKey}
			var args any
			switch {
			case len(c.Op) >= 9 && c.Op[:9] == "calendar.":
				ca, _ := contracts.ValidateCalendar(c)
				action.CalendarArgs = &ca
			case len(c.Op) >= 6 && c.Op[:6] == "notes.":
				na, _ := contracts.ValidateNoteCommand(c)
				action.NoteArgs = &na
				action.Args = nil
			case len(c.Op) >= 9 && c.Op[:9] == "messages.":
				m, _ := contracts.ValidateMessagesCommand(c)
				m.Files = nil
				if in.Context.ResendKey != "" {
					m.ResendOf = in.Context.ResendKey
				}
				action.Messages = &contracts.NativeMessagesArgs{MessagesArgs: m, Files: in.Context.Files}
			case len(c.Op) >= 9 && c.Op[:9] == "contacts.":
				action.Args = c.Args
			case len(c.Op) >= 6 && c.Op[:6] == "files.":
				args = &contracts.FileArgs{}
			case contracts.CaptureOperation(c.Op):
				args = &contracts.CaptureArgs{}
			case len(c.Op) >= 4 && c.Op[:4] == "mac.":
				args = &contracts.MacArgs{}
			case c.Op == "focus.set":
				args = &contracts.FocusSetArgs{}
			case c.Op == "shortcuts.run":
				args = &contracts.ShortcutRunArgs{}
			case contracts.PlaybackOperation(c.Op):
				args = &contracts.PlaybackArgs{}
			}
			if args != nil {
				if err = json.Unmarshal(c.Args, args); err != nil {
					panic(err)
				}
				action.Args = args
			}
			if contracts.CollectionSource(c.Op) != contracts.IPhone {
				raw, e := json.Marshal(action)
				if e != nil {
					panic(e)
				}
				row["action"] = string(raw)
				frame, _ := json.Marshal(contracts.IPCFrame{T: "Prepare", Origin: in.Context.Origin, CommandID: c.ID, Principal: "token:fixture", RequestHash: "hash_fixture", Action: &action, ExpiresAt: time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC), OwnerFence: 7})
				row["prepare"] = string(frame)
			}
		}
		out = append(out, row)
	}
	b, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		panic(err)
	}
	fmt.Println(string(b))
}
