package scenarios

import "testing"

func TestMessagesWriteChecksRequireCleanupBeforeSending(t *testing.T) {
	for _, id := range []string{"MSG-READSTATE-REAL", "MSG-DELETE-REAL", "MSG-RESEND", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING"} {
		e, allowed := nativeMessagesCleanupGate(id, Evidence{}, false)
		if allowed || e.Status != "environment-limited" || e.Metrics["writes_submitted"] != 0 || len(e.Assertions) != 1 || e.Assertions[0].Name != "cleanup_accessibility" {
			t.Fatal(id, e, allowed)
		}
		if _, allowed := nativeMessagesCleanupGate(id, Evidence{}, true); !allowed {
			t.Fatal(id, "cleanup-ready self-chat write was refused")
		}
	}
	if _, allowed := nativeMessagesCleanupGate("WEB-MSG-REAL-READ", Evidence{}, false); !allowed {
		t.Fatal("read-only web check requires a write permission")
	}
}
