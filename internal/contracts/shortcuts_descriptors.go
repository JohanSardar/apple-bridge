package contracts

import (
	"encoding/base64"
	"encoding/json"
	"strings"
)

type ShortcutRunArgs struct {
	Shortcut    string `json:"shortcut"`
	Input       string `json:"input,omitempty"`
	InputBase64 string `json:"input_base64,omitempty"`
	TimeoutMS   int    `json:"timeout_ms,omitempty"`
}

func ValidateShortcutRun(c Command) error {
	var a ShortcutRunArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil || strings.TrimSpace(a.Shortcut) == "" || len(a.Shortcut) > 1024 || strings.HasPrefix(a.Shortcut, "-") || len(a.Input) > 16<<10 || a.TimeoutMS < 0 || a.TimeoutMS > 300000 {
		return ErrInvalid
	}
	if a.InputBase64 != "" {
		data, err := base64.StdEncoding.DecodeString(a.InputBase64)
		if err != nil || len(data) > 16<<10 || a.Input != "" {
			return ErrInvalid
		}
	}
	return nil
}

func init() {
	RegisterSource(Shortcuts, "Shortcuts", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "shortcuts.run", Version: 1, Kind: CommandOp, Source: Shortcuts, Scope: "shortcuts:write", Capability: "shortcuts.run", Input: Command{}, Output: CommandRecord{}, Description: "Run a shortcut with text input; evidence establishes CLI completion only"},
			{Name: "shortcuts.actions", Version: 1, Kind: QueryOp, Source: Shortcuts, Scope: "shortcuts:read", Input: Query{}, Output: QueryResult{}, Description: "Read bounded installed App Intents metadata; no executable schema guarantee"},
		}
	}, 66, 90)
	messagesShape["shortcuts.folder"] = map[string]fieldShape{"name": requiredField(shapeString), "identifier": requiredField(shapeString)}
	messagesShape["shortcuts.shortcut"] = map[string]fieldShape{"name": requiredField(shapeString), "identifier": requiredField(shapeString), "folder_id": nullableField(shapeString), "accepts_input": nullableField(shapeBool)}
}

func ShortcutCompletionEvidence(raw json.RawMessage, hash string) bool {
	var result struct {
		ExitStatus *int   `json:"exit_status"`
		TimedOut   bool   `json:"timed_out"`
		Output     string `json:"output_base64"`
		Scope      string `json:"effect_scope"`
		Prepared   struct {
			RequestHash string `json:"request_hash"`
		} `json:"prepared"`
	}
	if json.Unmarshal(raw, &result) != nil {
		return false
	}
	output, err := base64.StdEncoding.DecodeString(result.Output)
	return err == nil && len(output) <= 32<<10 && result.ExitStatus != nil && *result.ExitStatus == 0 && !result.TimedOut && result.Scope == "runner completion only" && hash != "" && result.Prepared.RequestHash == hash
}
