package scenarios

import "testing"

func TestMessagesWriteChecksRequireCleanupBeforeSending(t *testing.T) {
	for _, id := range []string{"MSG-READSTATE-REAL", "MSG-DELETE-REAL", "MSG-RESEND", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING"} {
		e, allowed := nativeMessagesCleanupGate(id, Evidence{}, false, true)
		if allowed || e.Status != "environment-limited" || e.Metrics["writes_submitted"] != 0 || len(e.Assertions) != 1 || e.Assertions[0].Name != "cleanup_accessibility" {
			t.Fatal(id, e, allowed)
		}
		if e, allowed := nativeMessagesCleanupGate(id, Evidence{}, true, false); allowed || e.Metrics["writes_submitted"] != 0 || e.Assertions[0].Name != "cleanup_unlocked" {
			t.Fatal(id, "locked-Mac write check was allowed", e)
		}
		if _, allowed := nativeMessagesCleanupGate(id, Evidence{}, true, true); !allowed {
			t.Fatal(id, "cleanup-ready self-chat write was refused")
		}
	}
	if _, allowed := nativeMessagesCleanupGate("WEB-MSG-REAL-READ", Evidence{}, false, true); !allowed {
		t.Fatal("read-only web check requires a write permission")
	}
}
