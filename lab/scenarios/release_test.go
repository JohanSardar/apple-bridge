package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestReleaseDMGQualification(t *testing.T) {
	for _, test := range []struct {
		name       string
		assessment string
		codesign   string
		detach     string
		passed     bool
	}{
		{"notarized", "accepted\nsource=Notarized Developer ID", "0", "0", true},
		{"unnotarized", "accepted\nsource=Developer ID", "0", "0", false},
		{"invalid_app", "accepted\nsource=Notarized Developer ID", "1", "0", false},
		{"detach_failed", "accepted\nsource=Notarized Developer ID", "0", "1", false},
	} {
		t.Run(test.name, func(t *testing.T) {
			bin := t.TempDir()
			commands := map[string]string{
				"spctl":    "printf '%s\\n' '" + test.assessment + "'",
				"codesign": "exit " + test.codesign,
				"xcrun":    "exit 0",
				"hdiutil":  "if [ \"$1\" = detach ]; then /bin/rmdir \"$2/Apple Bridge.app\"; /bin/rm \"$2/Applications\"; exit " + test.detach + "; fi\n/bin/mkdir -p \"$5/Apple Bridge.app\"\n/bin/ln -s /Applications \"$5/Applications\"",
			}
			for name, body := range commands {
				if err := os.WriteFile(filepath.Join(bin, name), []byte("#!/bin/sh\n"+body+"\n"), 0700); err != nil {
					t.Fatal(err)
				}
			}
			t.Setenv("PATH", bin)
			t.Setenv("APPLE_BRIDGE_RELEASE_DMG", "fixture.dmg")
			result := runRelease(context.Background(), "real", Evidence{})
			if result.Passed != test.passed {
				t.Fatalf("passed=%t, want %t: %+v", result.Passed, test.passed, result.Assertions)
			}
		})
	}
}
