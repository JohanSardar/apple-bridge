package contracts

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"path/filepath"
	"strings"
)

type MessageFile struct {
	Name     string `json:"name"`
	UploadID string `json:"upload_id,omitempty"`
	Data     string `json:"data,omitempty"`
}
type NativeMessageFile struct {
	Name   string `json:"name"`
	Path   string `json:"path"`
	Digest string `json:"digest"`
}
type NativeMessagesArgs struct {
	MessagesArgs
	Files []NativeMessageFile `json:"files,omitempty"`
}

type MessagesArgs struct {
	ResendOf  string        `json:"resend_of,omitempty"`
	ID        string        `json:"id,omitempty"`
	ChatID    string        `json:"chat_id,omitempty"`
	Handles   []string      `json:"handles,omitempty"`
	Service   string        `json:"service,omitempty"`
	Text      string        `json:"text,omitempty"`
	Files     []MessageFile `json:"files,omitempty"`
	PartIndex int           `json:"part_index,omitempty"`
	Reaction  string        `json:"reaction,omitempty"`
	Emoji     string        `json:"emoji,omitempty"`
	Remove    bool          `json:"remove,omitempty"`
	Name      string        `json:"name,omitempty"`
	Handle    string        `json:"handle,omitempty"`
}

var MessagesOperations = []string{"messages.send", "messages.reply", "messages.react", "messages.edit", "messages.unsend", "messages.mark_read", "messages.mark_unread", "messages.delete_message", "messages.delete_chat", "messages.create_group", "messages.rename_group", "messages.add_member", "messages.remove_member", "messages.leave_group"}

func MessagesArgumentFields(op string) []string {
	switch op {
	case "messages.send":
		return []string{"chat_id", "handles", "service", "text", "files", "resend_of"}
	case "messages.reply":
		return []string{"id", "part_index", "text", "files"}
	case "messages.react":
		return []string{"id", "part_index", "reaction", "emoji", "remove"}
	case "messages.edit":
		return []string{"id", "part_index", "text"}
	case "messages.unsend":
		return []string{"id", "part_index"}
	case "messages.create_group":
		return []string{"handles", "service", "text", "name"}
	case "messages.rename_group":
		return []string{"id", "name"}
	case "messages.add_member", "messages.remove_member":
		return []string{"id", "handle"}
	default:
		return []string{"id"}
	}
}
func ValidateMessagesCommand(c Command) (MessagesArgs, error) {
	var a MessagesArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return a, ErrInvalid
	}
	var fields map[string]json.RawMessage
	json.Unmarshal(c.Args, &fields)
	allowed := map[string]bool{}
	for _, f := range MessagesArgumentFields(c.Op) {
		allowed[f] = true
	}
	for k, v := range fields {
		if !allowed[k] || bytes.Equal(bytes.TrimSpace(v), []byte("null")) {
			return a, ErrInvalid
		}
	}
	if a.PartIndex < 0 || len(a.Files) > MessageFileCount || len(a.Text) > 65536 {
		return a, ErrInvalid
	}
	for _, file := range a.Files {
		data, err := base64.StdEncoding.DecodeString(file.Data)
		if int64(len(data)) > MessageFileLimit {
			return a, ErrTooLarge
		}
		if err != nil || (file.Data == "") == (file.UploadID == "") || file.Name == "" || file.Name == "." || file.Name == ".." || filepath.Base(file.Name) != file.Name {
			return a, ErrInvalid
		}
	}
	switch c.Op {
	case "messages.send":
		if a.ResendOf != "" && a.ChatID == "" {
			return a, ErrInvalid
		}
		if a.ChatID != "" && a.Service != "" || (a.ChatID == "") == (len(a.Handles) == 0) || a.Text == "" && len(a.Files) == 0 || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
		if len(a.Handles) > 0 && (len(a.Handles) != 1 || a.Service != "iMessage" && a.Service != "SMS" && a.Service != "RCS") {
			return a, ErrInvalid
		}
	case "messages.create_group":
		if len(a.Handles) < 2 || a.Service == "" || a.Text == "" || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
	default:
		if a.ID == "" {
			return a, ErrInvalid
		}
		if c.Op == "messages.edit" || c.Op == "messages.unsend" || strings.HasPrefix(c.Op, "messages.delete_") {
			if c.ExpectedVersion == 0 {
				return a, ErrInvalid
			}
		}
	}
	if c.Op == "messages.edit" || c.Op == "messages.reply" {
		if a.Text == "" {
			return a, ErrInvalid
		}
	}
	if c.Op == "messages.rename_group" && a.Name == "" {
		return a, ErrInvalid
	}
	if (c.Op == "messages.add_member" || c.Op == "messages.remove_member") && a.Handle == "" {
		return a, ErrInvalid
	}
	if c.Op == "messages.react" {
		switch a.Reaction {
		case "love", "like", "dislike", "laugh", "emphasize", "question":
		case "emoji":
			if a.Emoji == "" {
				return a, ErrInvalid
			}
		default:
			return a, ErrInvalid
		}
	}
	return a, nil
}
func MessagesTargetCollection(op string) string {
	switch op {
	case "messages.mark_read", "messages.mark_unread", "messages.delete_chat", "messages.rename_group", "messages.add_member", "messages.remove_member", "messages.leave_group", "messages.create_group":
		return "messages.chat"
	}
	return "messages.message"
}
func init() {
	for _, op := range MessagesOperations {
		Operations = append(Operations, OperationDescriptor{Name: op, Version: 1, Kind: CommandOp, Source: Messages, Scope: "messages:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Messages command with evidence; unproven native routes reject before dispatch"})
	}
}
