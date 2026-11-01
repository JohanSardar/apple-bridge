package scenarios

import (
	"errors"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestNotesInitialMetadataCaptureWaitsForLiveNotesFolder(t *testing.T) {
	reads := 0
	err := notesAwaitCapture(func(limit time.Duration, ready func() bool) error {
		if limit > 30*time.Second {
			t.Fatal("initial metadata wait widened the scenario budget")
		}
		for i := 0; i < 4; i++ {
			if ready() {
				return nil
			}
		}
		return errors.New("initial capture unavailable")
	}, func() ([]contracts.Record, error) {
		reads++
		switch reads {
		case 1:
			return nil, errors.New("gateway starting")
		case 2:
			return []contracts.Record{{Collection: "reminders.list"}}, nil
		case 3:
			return []contracts.Record{{Collection: "notes.folder", Deleted: true}}, nil
		default:
			return []contracts.Record{{Collection: "notes.folder"}}, nil
		}
	})
	if err != nil || reads != 4 {
		t.Fatalf("initial capture err=%v reads=%d", err, reads)
	}
}

func TestNotesInitialCaptureFailureDoesNotPermitFixtures(t *testing.T) {
	problem := errors.New("initial capture timed out")
	err := notesAwaitCapture(func(_ time.Duration, ready func() bool) error {
		if ready() {
			t.Fatal("empty source permitted fixture creation")
		}
		return problem
	}, func() ([]contracts.Record, error) { return nil, nil })
	if !errors.Is(err, problem) {
		t.Fatalf("lost startup failure: %v", err)
	}
}
