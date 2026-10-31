package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCalendarParityCannotPassWithoutEffectsAndCleanup(t *testing.T) {
	for _, missing := range []string{"invitation_self_attendee_only", "invitation_response_readback", "travel_time_readback", "attachment_add_readback", "attachment_remove_readback", "owned_calendar_cleanup", "structured_location_readback"} {
		t.Run(missing, func(t *testing.T) {
			proof := calendarParityProof{Passed: true, Checks: map[string]bool{
				"accessibility_granted": true, "calendar_granted": true, "structured_location_readback": true,
				"invitation_self_attendee_only": true, "invitation_response_readback": true, "travel_time_readback": true,
				"attachment_add_readback": true, "attachment_remove_readback": true, "owned_calendar_cleanup": true,
			}}
			delete(proof.Checks, missing)
			for _, check := range calendarParityAssertions(proof, true) {
				if check.Name == missing && !check.Passed {
					return
				}
			}
			t.Fatal("missing native effect was accepted")
		})
	}
}

func TestCalendarParityReportSupportsCountOnlyDiagnostics(t *testing.T) {
	var proof calendarParityProof
	if err := json.Unmarshal([]byte(`{"passed":false,"checks":{"owned_calendar_cleanup":true},"controls":{"own_account_identity_count":2,"Accept":false}}`), &proof); err != nil {
		t.Fatal(err)
	}
	if !proof.Checks["owned_calendar_cleanup"] || proof.Controls["own_account_identity_count"] != float64(2) {
		t.Fatal("native cleanup or diagnostic counts were lost")
	}
}

func TestCalendarParityNeverRequestsAdditionalPermissions(t *testing.T) {
	for _, name := range []string{"CalendarParityProof.swift", "CalendarParityProbe.swift"} {
		data, err := os.ReadFile(filepath.Join("..", "..", "native", "Sources", "AppleSession", name))
		if err != nil {
			t.Fatal(err)
		}
		for _, forbidden := range []string{"AEDeterminePermissionToAutomateTarget", "NSAppleScript", "NSAppleEventDescriptor", "requestFullAccess", "requestAccess", "AXIsProcessTrustedWithOptions"} {
			if strings.Contains(string(data), forbidden) {
				t.Fatalf("%s must use existing EventKit and Accessibility access; found %s", name, forbidden)
			}
		}
	}
}
