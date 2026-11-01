package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/focuspack"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type focusPackFixture struct {
	calls   int
	input   shortcut.Input
	result  shortcut.Result
	focused bool
}

func (f *focusPackFixture) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	f.calls++
	f.input = input
	var args focuspack.SetArgs
	if err := json.Unmarshal(input.Data, &args); err != nil {
		return shortcut.Result{}, err
	}
	if f.result.ExitStatus == 0 && !f.result.TimedOut {
		f.focused = args.On
	}
	return f.result, nil
}
func runFocusPack(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if level != "sim" && s.ID != "FOC-SET-REAL" && s.ID != "FOC-STATUS-REAL" {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "Existing idle and phone qualification pending"}
		return e
	}
	if level == "real" {
		return runFocusReal(ctx, s, e)
	}
	if level == "sim" {
		fixture := &focusPackFixture{}
		pack := focuspack.Pack{Runner: fixture, ReadStatus: func(context.Context) (focuspack.Status, error) {
			return focuspack.Status{Focused: &fixture.focused, Authorization: "authorized"}, nil
		}}
		prior := fixture.focused
		err := pack.Set(ctx, focuspack.SetArgs{Mode: "Do Not Disturb", On: true, Until: time.Now().Add(time.Minute).UTC().Format(time.RFC3339)})
		add("set_readback", err == nil && fixture.focused)
		add("bounded_pack", fixture.input.Shortcut == focuspack.Name && fixture.input.Timeout == 15*time.Second)
		add("restore_readback", pack.Set(ctx, focuspack.SetArgs{Mode: "Do Not Disturb", On: prior}) == nil && fixture.focused == prior)
		fixture.result.TimedOut = true
		add("timeout_not_success", errors.Is(pack.Set(ctx, focuspack.SetArgs{Mode: "Do Not Disturb", On: true}), focuspack.ErrUnqualified))
		pack.ReadStatus = func(context.Context) (focuspack.Status, error) {
			return focuspack.Status{Authorization: "not_determined"}, nil
		}
		before := fixture.calls
		add("unknown_status_never_dispatches", errors.Is(pack.Set(ctx, focuspack.SetArgs{Mode: "Do Not Disturb", On: true}), focuspack.ErrUnqualified) && fixture.calls == before)
		command := contracts.Command{ID: contracts.NewCommandID(), Op: "focus.set", Args: json.RawMessage(`{"mode":"Do Not Disturb","on":true}`)}
		_, err = contracts.ValidateCommand(command)
		add("set_contract", err == nil)
		command.Args = json.RawMessage(`{"mode":"Do Not Disturb"}`)
		_, err = contracts.ValidateCommand(command)
		add("explicit_state_required", err != nil)
		w, err := world.New(world.WithOptions(ctx, world.Options{Scope: "reminders:read,focus:read,focus:write,system:read"}), level, 0, 0)
		add("gateway", err == nil)
		if err != nil {
			os.Stderr.WriteString(err.Error() + "\n")
			return e
		}
		defer w.Close()
		authority, err := trust.Open(w.Gateway.Home)
		add("authority", err == nil)
		if err != nil {
			return e
		}
		for _, level := range []trust.AgentLevel{trust.AgentFull, trust.AgentApprove} {
			_, token, err := authority.CreateAgent("Bridge Test Focus", level, []contracts.Grant{"focus:write"})
			add("agent", err == nil)
			if err != nil {
				continue
			}
			endpoint := w.Gateway
			endpoint.Token = token
			client := scripted.Client{Endpoint: endpoint}
			var result contracts.CommandRecord
			err = client.Call(ctx, "focus.set", contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{"mode":"Do Not Disturb","on":true}`)}, &result)
			add("agent_requires_approval", err == nil && result.State == contracts.CommandAwaitingApproval)
		}
		dir, err := os.MkdirTemp("", "ab-focus-build-")
		add("build_directory", err == nil)
		if err != nil {
			return e
		}
		defer os.RemoveAll(dir)
		output, err := exec.CommandContext(ctx, "go", "run", "./tools/shortcuts", "build", "-source", "packaging/shortcuts/ab-focus.json", "-out", dir, "-unsigned").CombinedOutput()
		add("pack_compiles", err == nil)
		if err != nil {
			os.Stderr.Write(output)
			return e
		}
		compiledJSON, err := exec.CommandContext(ctx, "plutil", "-convert", "json", "-o", "-", filepath.Join(dir, "ab-focus.unsigned.shortcut")).Output()
		var compiled struct {
			Actions []struct {
				Identifier string `json:"WFWorkflowActionIdentifier"`
				Parameters struct {
					UUID            string
					DictionaryKey   string `json:"WFDictionaryKey"`
					ControlFlowMode int    `json:"WFControlFlowMode"`
					Input           struct {
						Type     string
						Variable struct {
							SerializationType string `json:"WFSerializationType"`
							Value             struct {
								Type       string
								OutputUUID string
							}
						}
					} `json:"WFInput"`
				} `json:"WFWorkflowActionParameters"`
			} `json:"WFWorkflowActions"`
		}
		if err == nil {
			err = json.Unmarshal(compiledJSON, &compiled)
		}
		add("compiled_pack_read", err == nil)
		until := ""
		for _, action := range compiled.Actions {
			if action.Identifier == "is.workflow.actions.getvalueforkey" && action.Parameters.DictionaryKey == "until" {
				until = action.Parameters.UUID
			}
		}
		bound, conditions := true, 0
		for _, action := range compiled.Actions {
			if action.Identifier != "is.workflow.actions.conditional" || action.Parameters.ControlFlowMode != 0 {
				continue
			}
			conditions++
			input := action.Parameters.Input
			bound = bound && input.Type == "Variable" && input.Variable.SerializationType == "WFTextTokenAttachment" && input.Variable.Value.Type == "ActionOutput" && until != "" && input.Variable.Value.OutputUUID == until
		}
		add("pack_conditional_input_bound", bound && conditions == 1)
	}
	build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=FocusChecks")
	output, err := build.CombinedOutput()
	add("native_build", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	raw, err := os.ReadFile("native/.build/native-output-1")
	add("native_output", err == nil)
	if err != nil {
		return e
	}
	mode := level
	if s.ID == "FOC-STATUS-REAL" {
		mode = "status"
	}
	output, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(raw)), "FocusChecks"), mode).CombinedOutput()
	if err == nil && strings.HasPrefix(string(output), "limited:") {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": strings.TrimSpace(strings.TrimPrefix(string(output), "limited:"))}
		return e
	}
	add("native_set_readback_restore", err == nil && strings.TrimSpace(string(output)) == "passed")
	if err != nil {
		os.Stderr.Write(output)
	}
	return e
}
