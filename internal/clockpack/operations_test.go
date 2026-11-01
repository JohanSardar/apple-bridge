package clockpack

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type exchangeFixture struct {
	data  string
	calls int
}

type refreshingExchangeFixture struct {
	mutation string
	fresh    string
	calls    []string
}

func (f *refreshingExchangeFixture) Run(_ context.Context, input shortcut.Input) (shortcut.Result, error) {
	var request Request
	if err := json.Unmarshal(input.Data, &request); err != nil {
		return shortcut.Result{}, err
	}
	f.calls = append(f.calls, request.Operation)
	data := f.mutation
	if request.Operation == "alarms" {
		data = f.fresh
	}
	return shortcut.Result{Data: []byte(data)}, nil
}

func TestAlarmMutationRechecksStaleEntityResultsInANewInvocation(t *testing.T) {
	for _, operation := range []string{"toggle_alarm", "delete_alarm"} {
		t.Run(operation, func(t *testing.T) {
			fixture := &refreshingExchangeFixture{
				mutation: `{"version":1,"operation":"` + operation + `","alarms":[{"id":"one","label":"[ab-test] synthetic","time":"07:05","repeat_days":[1,3],"enabled":true}]}`,
				fresh:    `{"version":1,"operation":"alarms","alarms":[]}`,
			}
			if operation == "toggle_alarm" {
				fixture.fresh = `{"version":1,"operation":"alarms","alarms":[{"id":"one","label":"[ab-test] synthetic","time":"07:05","repeat_days":[1,3],"enabled":false}]}`
			}
			request := Request{Operation: operation, Alarm: &Alarm{ID: "one", Label: "[ab-test] synthetic", Time: "07:05", RepeatDays: []int{1, 3}}}
			out, err := (Pack{Runner: fixture}).Exchange(context.Background(), request)
			if err != nil || out.Operation != operation || len(fixture.calls) != 2 || fixture.calls[1] != "alarms" {
				t.Fatal(out, err, fixture.calls)
			}
			fixture.calls = nil
			fixture.fresh = `{"version":1,"operation":"alarms","alarms":[{"id":"one","label":"[ab-test] synthetic","time":"07:05","repeat_days":[1,3],"enabled":true}]}`
			if _, err := (Pack{Runner: fixture}).Exchange(context.Background(), request); !errors.Is(err, ErrUnqualified) || len(fixture.calls) != 2 {
				t.Fatal("a fresh readback must still prove the requested change", err, fixture.calls)
			}
		})
	}
}

func (f *exchangeFixture) Run(context.Context, shortcut.Input) (shortcut.Result, error) {
	f.calls++
	return shortcut.Result{Data: []byte(f.data)}, nil
}

func TestTimerReadbackRequiresEveryFieldAndUniqueIDs(t *testing.T) {
	for _, data := range []string{
		`{"version":1,"operation":"timers","timers":null}`,
		`{"version":1,"operation":"timers","timers":[{"id":"one","duration":120,"running":true}]}`,
		`{"version":1,"operation":"timers","timers":[{"id":"one","label":"test","duration":120}]}`,
		`{"version":1,"operation":"timers","timers":[{"id":"one","label":null,"duration":120,"running":true}]}`,
		`{"version":1,"operation":"timers","timers":[{"id":"one","label":"test","duration":120,"running":true},{"id":"one","label":"test","duration":120,"running":true}]}`,
	} {
		_, err := (Pack{Runner: &exchangeFixture{data: data}}).Exchange(context.Background(), Request{Operation: "timers"})
		if !errors.Is(err, ErrUnqualified) {
			t.Fatal(data, err)
		}
	}
	for _, data := range []string{
		`{"version":1,"operation":"timers","timers":[]}`,
		`{"version":1,"operation":"timers","timers":[{"id":"one","label":"test","duration":120,"running":false}]}`,
	} {
		if _, err := (Pack{Runner: &exchangeFixture{data: data}}).Exchange(context.Background(), Request{Operation: "timers"}); err != nil {
			t.Fatal(data, err)
		}
	}
}

func TestTimerDeleteRequiresTagAndObservedAbsence(t *testing.T) {
	fixture := &exchangeFixture{data: `{"version":1,"operation":"delete_timer","timers":[{"id":"one","label":"[ab-test] run","duration":120,"running":true}]}`}
	pack := Pack{Runner: fixture}
	_, err := pack.Exchange(context.Background(), Request{Operation: "delete_timer", Timer: &Timer{ID: "one", Label: "personal"}})
	if err == nil || fixture.calls != 0 {
		t.Fatal(err, fixture.calls)
	}
	_, err = pack.Exchange(context.Background(), Request{Operation: "delete_timer", Timer: &Timer{ID: "one", Label: "[ab-test] run"}})
	if !errors.Is(err, ErrUnqualified) {
		t.Fatal(err)
	}
	fixture.data = `{"version":1,"operation":"delete_timer","timers":[]}`
	if _, err := pack.Exchange(context.Background(), Request{Operation: "delete_timer", Timer: &Timer{ID: "one", Label: "[ab-test] run"}}); err != nil {
		t.Fatal(err)
	}
}

func TestAlarmReadbackNormalizesNativeTimeAndUnorderedRepeatDays(t *testing.T) {
	fixture := &exchangeFixture{data: `{"version":1,"operation":"alarms","alarms":[{"id":"one","label":"synthetic","time":"7:5","repeat_days":[3,1],"enabled":true}]}`}
	got, err := (Pack{Runner: fixture}).Exchange(context.Background(), Request{Operation: "alarms"})
	if err != nil || len(got.Alarms) != 1 || got.Alarms[0].Time != "07:05" || got.Alarms[0].RepeatDays[0] != 1 || got.Alarms[0].RepeatDays[1] != 3 {
		t.Fatal(got, err)
	}
	fixture.data = `{"version":1,"operation":"alarms","alarms":[{"id":"one","label":"synthetic","time":"7:5","repeat_days":[1,1],"enabled":true}]}`
	if _, err = (Pack{Runner: fixture}).Exchange(context.Background(), Request{Operation: "alarms"}); !errors.Is(err, ErrUnqualified) {
		t.Fatal(err)
	}
}

func TestCreatedAlarmRepeatDaysHaveSetSemantics(t *testing.T) {
	fixture := &exchangeFixture{data: `{"version":1,"operation":"create_alarm","alarms":[{"id":"assigned","label":"[ab-test] synthetic","time":"7:5","repeat_days":[1,3],"enabled":true}]}`}
	alarm := Alarm{ID: "requested", Label: "[ab-test] synthetic", Time: "07:05", RepeatDays: []int{3, 1}, Enabled: true}
	if _, err := (Pack{Runner: fixture}).Exchange(context.Background(), Request{Operation: "create_alarm", Alarm: &alarm}); err != nil {
		t.Fatal(err)
	}
	if alarm.RepeatDays[0] != 3 {
		t.Fatal("readback validation changed caller-owned weekdays")
	}
}

func TestRecentTimerReadbackAcceptsNativeResponseWithoutAlarmFields(t *testing.T) {
	fixture := &exchangeFixture{data: `{"version":1,"operation":"recent_timers","capabilities":["timers","start_timer","delete_timer"],"timers":[{"id":"recent:1","label":"[ab-test] synthetic","duration":900,"running":false}]}`}
	out, err := (Pack{Runner: fixture}).Exchange(context.Background(), Request{Operation: "recent_timers"})
	if err != nil || len(out.Timers) != 1 || out.Timers[0].ID != "recent:1" {
		t.Fatal(out, err)
	}
}
