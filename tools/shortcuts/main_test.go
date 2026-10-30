package main

import "testing"

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
