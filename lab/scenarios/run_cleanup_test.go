package scenarios

import (
	"context"
	"testing"
	"time"
)

func TestRunCleanup(t *testing.T) {
	residue := cleanupItem{ID: "old", Tag: "[ab-test old]", Container: "self"}
	owned := cleanupItem{ID: "new", Tag: "[ab-test run]", Container: "self"}
	c, err := newRunCleanup("run", []cleanupItem{residue})
	if err != nil {
		t.Fatal(err)
	}
	if c.capture(residue) == nil {
		t.Fatal("accepted existing residue")
	}
	if err := c.capture(owned); err != nil {
		t.Fatal(err)
	}
	rows := []cleanupItem{residue, owned}
	proof, err := c.finish(func() ([]cleanupItem, error) { return rows, nil }, func(item cleanupItem) error {
		if item != owned {
			t.Fatal("deleted residue")
		}
		rows = rows[:1]
		return nil
	})
	if err != nil || proof["passed"] != true {
		t.Fatal(proof, err)
	}
	rows = []cleanupItem{residue, {ID: "new", Tag: owned.Tag, Container: "other"}}
	if _, err := c.finish(func() ([]cleanupItem, error) { return rows, nil }, func(cleanupItem) error { t.Fatal("deleted changed identity"); return nil }); err == nil {
		t.Fatal("accepted changed container")
	}
	rows = nil
	if _, err := c.finish(func() ([]cleanupItem, error) { return rows, nil }, func(cleanupItem) error { return nil }); err == nil {
		t.Fatal("lost baseline accepted")
	}
}

func TestRunCleanupWaitsForNativeReadback(t *testing.T) {
	old := cleanupItem{ID: "old", Container: "self"}
	added := cleanupItem{ID: "new", Tag: "[ab-test run]", Container: "self"}
	c, _ := newRunCleanup("run", []cleanupItem{old})
	c.settle = time.Second
	if err := c.capture(added); err != nil {
		t.Fatal(err)
	}
	reads := 0
	proof, err := c.finish(func() ([]cleanupItem, error) {
		reads++
		if reads < 3 {
			return []cleanupItem{old, added}, nil
		}
		return []cleanupItem{old}, nil
	}, func(cleanupItem) error { return nil })
	if err != nil || proof["passed"] != true {
		t.Fatal(proof, err)
	}
}

func TestUnqualifiedAttachmentCleanupDoesNotStartWorld(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	for _, id := range []string{"WEB-UPLOAD-NATIVE-STAGING"} {
		e := runNativeMessagesCommands(ctx, Scenario{ID: id}, "real", Evidence{})
		if e.Status != "limited" || e.Metrics["writes_submitted"] != 0 || len(e.Assertions) != 1 || e.Assertions[0].Name != "owned_attachment_cleanup_qualified" {
			t.Fatal(id, e)
		}
	}
}
