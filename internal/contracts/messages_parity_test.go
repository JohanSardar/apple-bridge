package contracts

import (
	"encoding/json"
	"reflect"
	"slices"
	"testing"
)

func TestMessagesParityContract(t *testing.T) {
	for _, tc := range []struct {
		op, args string
		valid    bool
	}{
		{"pin", `{"id":"chat","pinned_index":0}`, true},
		{"pin", `{"id":"chat","pinned_index":-1}`, true},
		{"pin", `{"id":"chat","pinned_index":9}`, false},
		{"pin", `{"id":"chat","pinned_index":null}`, false},
		{"pin", `{"id":"chat"}`, false},
		{"mute", `{"id":"chat","muted":false}`, true},
		{"mute", `{"id":"chat"}`, false},
		{"set_read_receipts", `{"id":"chat","read_receipts":"default"}`, true},
		{"set_read_receipts", `{"id":"chat","read_receipts":"off"}`, true},
		{"set_read_receipts", `{"id":"chat","read_receipts":"maybe"}`, false},
		{"recover_chat", `{"id":"chat"}`, true},
		{"delete_chat_permanently", `{"id":"chat","all":true}`, false},
	} {
		t.Run(tc.op+tc.args, func(t *testing.T) {
			c := Command{ID: NewID("cmd"), Op: "messages." + tc.op, Args: json.RawMessage(tc.args), ExpectedVersion: 1}
			_, err := ValidateMessagesCommand(c)
			if (err == nil) != tc.valid {
				t.Fatalf("valid=%v err=%v", tc.valid, err)
			}
			if MessagesTargetCollection(c.Op) != "messages.chat" {
				t.Fatal("chat target")
			}
			c.ExpectedVersion = 0
			if _, err = ValidateMessagesCommand(c); err == nil {
				t.Fatal("version required")
			}
		})
	}
}

func TestMessagesParitySchema(t *testing.T) {
	count := 0
	for _, descriptor := range Operations {
		if !IsMessagesParity(descriptor.Name) {
			continue
		}
		count++
		schema := OperationSchema(descriptor)
		required := schema["required"].([]string)
		if !slices.Contains(required, "expected_version") {
			t.Fatal(descriptor.Name, "missing version precondition")
		}
		args := schema["properties"].(map[string]any)["args"].(map[string]any)
		if !reflect.DeepEqual(args["required"], MessagesParityFields(descriptor.Name)) {
			t.Fatal(descriptor.Name, "argument requirements")
		}
		fields := args["properties"].(map[string]any)
		for _, name := range MessagesParityFields(descriptor.Name) {
			if fields[name].(map[string]any)["nullable"] == true {
				t.Fatal(name, "required argument accepts null")
			}
		}
		if descriptor.Name == "messages.pin" {
			position := fields["pinned_index"].(map[string]any)
			if position["minimum"] != -1 || position["maximum"] != 8 {
				t.Fatal("pin range")
			}
		}
		if descriptor.Name == "messages.set_read_receipts" && !reflect.DeepEqual(fields["read_receipts"].(map[string]any)["enum"], []string{"on", "off", "default"}) {
			t.Fatal("receipt values")
		}
	}
	if count != 5 {
		t.Fatalf("registered parity operations: %d", count)
	}
}
