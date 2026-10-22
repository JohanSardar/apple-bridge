package scenarios

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestNativeFirstVisibilitySurvivesLaterUpdates(t *testing.T) {
	record := contracts.Record{ID: "fixture", Collection: "reminders.reminder", Data: json.RawMessage(`{"title":"[ab-test] visibility"}`)}
	first := Seen{Change: contracts.Change{Op: "upsert", Record: record}, At: time.Unix(1, 0), UptimeNS: 100}
	latest := first
	latest.At = time.Unix(41, 0)
	latest.UptimeNS = 40100
	observer := watcher{seen: map[string]Seen{key(record): latest}, changes: []Seen{first, latest}}
	got, ok := observer.firstObservedExact("[ab-test] visibility")
	if !ok || got.UptimeNS != first.UptimeNS {
		t.Fatal("first visibility was replaced by a later update")
	}
	got, ok = observer.observedExact("[ab-test] visibility")
	if !ok || got.UptimeNS != latest.UptimeNS {
		t.Fatal("current-record observer lost the latest update")
	}
}
