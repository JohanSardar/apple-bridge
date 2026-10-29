package contracts

import (
	"bytes"
	"encoding/json"
	"errors"
	"reflect"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestQueryResultFrameCapacity(t *testing.T) {
	for _, character := range []string{"x", "<"} {
		result := json.RawMessage(`{"value":"` + strings.Repeat(character, MaxQueryJSON-12) + `"}`)
		if len(result) != MaxQueryJSON {
			t.Fatal("incorrect boundary fixture")
		}
		frame := IPCFrame{T: "QueryResult", ID: "qry_boundary", ResultJSON: result}
		encoded, err := EncodeFrame(frame)
		if err != nil {
			t.Fatal(err)
		}
		var decoded IPCFrame
		if _, err = ReadFrame(bytes.NewReader(encoded), &decoded); err != nil || !bytes.Equal(decoded.ResultJSON, result) {
			t.Fatal("inline result did not round trip", err)
		}
		if decoded.Blobs == nil {
			t.Fatal("query blobs must be an array")
		}
		resultJSON, err := MarshalQueryJSON(QueryResult{ID: "qry_boundary", ResultJSON: result})
		if err != nil {
			t.Fatal(err)
		}
		feed, err := MarshalQueryJSON(FeedFrame{T: "CommandResponse", Response: &CommandResponse{Result: resultJSON}})
		if err != nil || len(feed) > MaxQueryFrame {
			t.Fatal("forwarded query exceeded wire capacity", len(feed), err)
		}
		var forwarded FeedFrame
		var received QueryResult
		if json.Unmarshal(feed, &forwarded) != nil || json.Unmarshal(forwarded.Response.Result, &received) != nil || !bytes.Equal(received.ResultJSON, result) {
			t.Fatal("forwarded query changed JSON")
		}
		frame.T = "Capture"
		if _, err = EncodeFrame(frame); !errors.Is(err, ErrCapacity) {
			t.Fatal("capture frame limit changed", err)
		}
	}
}

func TestQueryEncodingAllocationBound(t *testing.T) {
	raw := json.RawMessage(`"` + strings.Repeat("x", 32<<20) + `"`)
	for name, encode := range map[string]func(any) ([]byte, error){"json": MarshalQueryJSON, "frame": EncodeFrame} {
		t.Run(name, func(t *testing.T) {
			frame := IPCFrame{T: "QueryResult", ID: "qry_large", ResultJSON: raw}
			var before, after runtime.MemStats
			runtime.ReadMemStats(&before)
			_, err := encode(frame)
			runtime.ReadMemStats(&after)
			if !errors.Is(err, ErrCapacity) {
				t.Fatalf("oversized result: %v", err)
			}
			if allocated := after.TotalAlloc - before.TotalAlloc; allocated > 4<<20 {
				t.Fatalf("oversized result allocated %d bytes", allocated)
			}
		})
	}
}

func TestForwardQueryDeadline(t *testing.T) {
	for _, test := range []struct {
		raw  string
		want time.Duration
	}{
		{`{"method":"query","native_query":{"deadline_ms":30000}}`, 30 * time.Second},
		{`{"method":"query","native_query":{"deadline_ms":60000}}`, time.Minute},
		{`{"method":"query","native_query":{}}`, time.Second},
		{`{"method":"query","native_query":{"deadline_ms":60001}}`, 20 * time.Second},
		{`{"method":"submit"}`, 20 * time.Second},
	} {
		if got := ForwardTimeout(json.RawMessage(test.raw)); got != test.want {
			t.Fatalf("deadline %s: got %s want %s", test.raw, got, test.want)
		}
	}
}

func TestContactPrepareRawArgumentsRoundTrip(t *testing.T) {
	args := json.RawMessage(`{"given_name":"[ab-test wire]","phones":[{"label":"mobile","value":"+15555550123"}]}`)
	command := Command{ID: NewCommandID(), Op: "contacts.create", Args: args}
	validated, err := ValidateCommand(command)
	if err != nil {
		t.Fatal(err)
	}
	frame := IPCFrame{T: "Prepare", CommandID: command.ID, Principal: "device:test", RequestHash: "digest", Action: &NativeAction{Op: command.Op, Args: validated.Contact}, ExpiresAt: time.Now().Add(time.Minute)}
	encoded, err := EncodeFrame(frame)
	if err != nil {
		t.Fatal(err)
	}
	var decoded IPCFrame
	if _, err = ReadFrame(bytes.NewReader(encoded), &decoded); err != nil {
		t.Fatal(err)
	}
	actual, err := json.Marshal(decoded.Action.Args)
	if err != nil {
		t.Fatal(err)
	}
	var want, got any
	if json.Unmarshal(args, &want) != nil || json.Unmarshal(actual, &got) != nil || !reflect.DeepEqual(got, want) {
		t.Fatal("native arguments changed")
	}
}
