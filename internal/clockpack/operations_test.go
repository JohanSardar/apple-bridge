package clockpack

import (
	"context"
	"errors"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type exchangeFixture struct {
	data  string
	calls int
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
