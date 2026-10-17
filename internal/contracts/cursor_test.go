package contracts

import (
	"encoding/base64"
	"testing"
)

func TestEventCursorCodec(t *testing.T) {
	for _, sequence := range []uint64{0, 1, 18446744073709551615} {
		cursor := Cursor{Epoch: "epoch", SubscriptionRevision: 7, Sequence: sequence}
		decoded, err := ParseEventID(cursor.EventID())
		if err != nil || decoded != cursor {
			t.Fatalf("cursor round trip: %v %v", decoded, err)
		}
	}
	for _, raw := range []string{`{}`, `{"epoch":"epoch","subscription_revision":1}`, `{"epoch":"epoch","subscription_revision":1,"sequence":null}`, `{"epoch":"epoch","subscription_revision":1,"sequence":-1}`, `{"epoch":"epoch","subscription_revision":1,"sequence":1.5}`, `{"epoch":"epoch","subscription_revision":0,"sequence":1}`, `{"epoch":"epoch","subscription_revision":1,"sequence":1,"extra":true}`} {
		if _, err := ParseEventID(base64.RawURLEncoding.EncodeToString([]byte(raw))); err == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
	if _, err := ParseEventID("42"); err == nil {
		t.Fatal("accepted sequence without identity")
	}
}
