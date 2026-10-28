package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runContactsIdentity(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.NewContacts(ctx, level, 0, true, false)
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	if s.ID == "CON-NOTE" {
		home, err := os.UserHomeDir()
		raw, signErr := exec.CommandContext(ctx, "codesign", "-d", "--entitlements", ":-", filepath.Join(home, "Applications/AppleBridgeDev/AppleBridgeHelper.app")).Output()
		add("signed_helper_entitlements_checked", err == nil && signErr == nil && strings.Contains(string(raw), "<plist"))
		absent := !strings.Contains(string(raw), "com.apple.developer.contacts.notes")
		add("notes_entitlement_absent", absent)
		waitErr := w.Await(15*time.Second, func() bool {
			records, readErr := contactRecords(ctx, w, w.Gateway)
			if readErr != nil {
				return false
			}
			count := 0
			for _, r := range records {
				if r.Collection != "contacts.contact" {
					continue
				}
				count++
				var data map[string]any
				if json.Unmarshal(r.Data, &data) != nil || data["note"] != "unavailable" {
					return false
				}
			}
			return count > 0
		})
		add("notes_explicitly_unavailable", waitErr == nil)
		e.Status = "limited"
		e.Metrics = map[string]any{"reason": "CNContactNoteKey read/write requires com.apple.developer.contacts.notes; the signed Helper lacks this restricted entitlement"}
	} else {
		truth, err := w.Truth("observe", map[string]any{"source": "contacts", "me_only": true})
		expected, _ := truth["me_identifier_digest"].(string)
		add("independent_me_card_available", err == nil && len(expected) == 64)
		matched := false
		actual := ""
		waitErr := w.Await(15*time.Second, func() bool {
			records, err := contactRecords(ctx, w, w.Gateway)
			aliases, aliasErr := gatewayAliases(w)
			if err != nil || aliasErr != nil {
				return false
			}
			count := 0
			for _, r := range records {
				if r.Collection != "contacts.contact" {
					continue
				}
				var data map[string]any
				json.Unmarshal(r.Data, &data)
				if data["is_me"] != true {
					continue
				}
				count++
				for key, id := range aliases {
					if id == r.ID && strings.HasPrefix(key, "contacts/contacts.contact/") {
						sum := sha256.Sum256([]byte(strings.TrimPrefix(key, "contacts/contacts.contact/")))
						actual = hex.EncodeToString(sum[:])
					}
				}
			}
			matched = count == 1 && actual == expected && expected != ""
			return matched
		})
		add("me_card_matches_truth", waitErr == nil && matched)
		e.Metrics = map[string]any{"me_match": matched, "me_identifier_digest": actual}
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
