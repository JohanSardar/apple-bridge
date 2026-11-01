package main

import (
	"bytes"
	"encoding/json"
	"os/exec"
	"runtime"
	"testing"
)

func compiledActions(t *testing.T, source string) []struct {
	Identifier string         `json:"WFWorkflowActionIdentifier"`
	Parameters map[string]any `json:"WFWorkflowActionParameters"`
} {
	t.Helper()
	if runtime.GOOS != "darwin" {
		t.Skip("Apple's plist validation requires macOS")
	}
	raw, err := compile(source)
	if err != nil {
		t.Fatal(err)
	}
	command := exec.Command("/usr/bin/plutil", "-convert", "json", "-o", "-", "-")
	command.Stdin = bytes.NewReader(raw)
	data, err := command.Output()
	if err != nil {
		t.Fatal(err)
	}
	var workflow struct {
		Actions []struct {
			Identifier string         `json:"WFWorkflowActionIdentifier"`
			Parameters map[string]any `json:"WFWorkflowActionParameters"`
		} `json:"WFWorkflowActions"`
	}
	if err := json.Unmarshal(data, &workflow); err != nil {
		t.Fatal(err)
	}
	return workflow.Actions
}

func TestBundledEchoOutput(t *testing.T) {
	raw, err := compile("../../packaging/shortcuts/ab-echo.json")
	if err != nil || !bytes.Contains(raw, []byte("<key>WFWorkflowHasShortcutInputVariables</key>\n<true/>")) {
		t.Fatal("echo must declare its Shortcut Input variable to the importer", err)
	}
	actions := compiledActions(t, "../../packaging/shortcuts/ab-echo.json")
	if len(actions) != 2 || actions[0].Identifier != "is.workflow.actions.gettext" || actions[1].Identifier != "is.workflow.actions.output" {
		t.Fatal("echo must convert Shortcut Input to text and explicitly return it")
	}
	text := actions[0].Parameters["WFTextActionText"].(map[string]any)
	if text["WFSerializationType"] != "WFTextTokenString" {
		t.Fatal("Shortcut Input must be embedded in a text token string")
	}
	value := text["Value"].(map[string]any)
	attachment := value["attachmentsByRange"].(map[string]any)["{0, 1}"].(map[string]any)
	if value["string"] != "\ufffc" || attachment["Type"] != "ExtensionInput" {
		t.Fatal("echo lost its Shortcut Input attachment")
	}
	token := actions[1].Parameters["WFOutput"].(map[string]any)
	if token["WFSerializationType"] != "WFTextTokenString" {
		t.Fatal("Stop and Output requires a text token string")
	}
	output := token["Value"].(map[string]any)["attachmentsByRange"].(map[string]any)["{0, 1}"].(map[string]any)
	if output["Type"] != "ActionOutput" || output["OutputUUID"] != actions[0].Parameters["UUID"] {
		t.Fatal("echo output must reference the converted text")
	}
}

func TestBundledStatusOutput(t *testing.T) {
	actions := compiledActions(t, "../../packaging/shortcuts/ab-bridge-status.json")
	if len(actions) != 2 || actions[0].Identifier != "com.johansardar.applebridge.services.BridgeStatusIntent" || actions[1].Identifier != "is.workflow.actions.output" {
		t.Fatal("status wrapper must invoke the installed Helper intent and return its result")
	}
	descriptor := actions[0].Parameters["AppIntentDescriptor"].(map[string]any)
	if descriptor["BundleIdentifier"] != "com.johansardar.applebridge.services" || descriptor["AppIntentIdentifier"] != "BridgeStatusIntent" {
		t.Fatal("status wrapper must target the services-signed Helper")
	}
	token := actions[1].Parameters["WFOutput"].(map[string]any)
	if token["WFSerializationType"] != "WFTextTokenString" {
		t.Fatal("Stop and Output requires a text token string")
	}
	output := token["Value"].(map[string]any)["attachmentsByRange"].(map[string]any)["{0, 1}"].(map[string]any)
	if output["Type"] != "ActionOutput" || output["OutputUUID"] != actions[0].Parameters["UUID"] {
		t.Fatal("status output must come from the real intent")
	}
}

func TestTextOutputReference(t *testing.T) {
	output, err := resolve(map[string]any{"$output": "alarm", "$text": true, "$property": "label"}, "Clock", map[string]string{"alarm": "alarm-uuid"})
	if err != nil {
		t.Fatal(err)
	}
	token := output.(map[string]any)
	if token["WFSerializationType"] != "WFTextTokenString" {
		t.Fatal("text reference must use token string serialization")
	}
	value := token["Value"].(map[string]any)
	if value["string"] != "\ufffc" {
		t.Fatal("text reference must contain the attachment placeholder")
	}
	variable := value["attachmentsByRange"].(map[string]any)["{0, 1}"].(map[string]any)
	if variable["OutputUUID"] != "alarm-uuid" || variable["Type"] != "ActionOutput" {
		t.Fatal("text attachment lost its output identity")
	}
	property := variable["Aggrandizements"].([]any)[0].(map[string]any)
	if property["PropertyName"] != "label" {
		t.Fatal("text attachment lost its property")
	}
	if _, err := resolve(map[string]any{"$output": "missing", "$text": true}, "Clock", nil); err == nil {
		t.Fatal("unknown text output accepted")
	}
	if _, err := compile("../../packaging/shortcuts/ab-clock.json"); err != nil {
		t.Fatal(err)
	}
}

func TestConditionalStringCoercion(t *testing.T) {
	output, err := resolve(map[string]any{"$output": "target", "$property": "url", "$coerce": "WFStringContentItem"}, "Clock", map[string]string{"target": "target-uuid"})
	if err != nil {
		t.Fatal(err)
	}
	variable := output.(map[string]any)["Value"].(map[string]any)
	if variable["OutputUUID"] != "target-uuid" {
		t.Fatal("coercion lost its output binding")
	}
	aggrandizements := variable["Aggrandizements"].([]any)
	if len(aggrandizements) != 2 || aggrandizements[0].(map[string]any)["PropertyName"] != "url" {
		t.Fatal("coercion lost its property binding")
	}
	coercion := aggrandizements[1].(map[string]any)
	if coercion["Type"] != "WFCoercionVariableAggrandizement" || coercion["CoercionItemClass"] != "WFStringContentItem" {
		t.Fatal("conditional subject must explicitly coerce to text")
	}
}
