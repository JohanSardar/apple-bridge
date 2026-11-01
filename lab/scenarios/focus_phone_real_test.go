package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAirFocusEvidenceRequiresGrantBaselineSetAndRestore(t *testing.T) {
	for _, raw := range []string{
		`{`, `{}`, `{"grant_passed":true}`, `{"grant_passed":true,"inactive_passed":false}`,
		`{"grant_passed":true,"inactive_passed":true,"set_passed":false,"restore_passed":true}`,
		`{"grant_passed":true,"inactive_passed":true,"set_passed":true,"restore_passed":false}`,
		`{"grant_passed":true,"inactive_passed":true,"set_passed":true,"restore_passed":"true"}`,
	} {
		if airFocusEvidence(Evidence{Passed: true}, []byte(raw)).Passed {
			t.Fatalf("accepted incomplete Air proof: %s", raw)
		}
	}
	raw := []byte(`{"grant_passed":true,"inactive_passed":true,"set_passed":true,"restore_passed":true}`)
	e := airFocusEvidence(Evidence{Passed: true}, raw)
	if !e.Passed || len(e.Assertions) != 5 {
		t.Fatalf("rejected complete Air proof: %+v", e)
	}
	if airFocusEvidence(Evidence{Passed: false}, raw).Passed {
		t.Fatal("lost an earlier failure")
	}
}

func TestPhoneFocusUsesAirLockAndRemoteStep(t *testing.T) {
	dir := t.TempDir()
	log := filepath.Join(dir, "ssh.log")
	t.Setenv("AB_TEST_SSH_LOG", log)
	t.Setenv("PATH", dir+":"+os.Getenv("PATH"))
	script := `#!/bin/sh
printf '%s\n' "$*" >> "$AB_TEST_SSH_LOG"
case "$*" in
 *lockf*) printf 'locked\n'; cat >/dev/null ;;
 *) cat > "$AB_TEST_SSH_LOG.step"; printf '{"grant_passed":true,"inactive_passed":true,"set_passed":true,"restore_passed":true}' ;;
esac
`
	if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	release, err := lockAirFocus(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer release()
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	t.Chdir(filepath.Join(wd, "..", ".."))
	if e := runAirFocusReal(context.Background()); !e.Passed {
		t.Fatalf("remote proof failed: %+v", e)
	}
	raw, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), `-o BatchMode=yes air lockf -k "$HOME/.ab-locks/app-focus"`) || !strings.Contains(string(raw), "-o BatchMode=yes air sh -s") {
		t.Fatalf("wrong Air routing: %s", raw)
	}
	step, err := os.ReadFile(log + ".step")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(step), "FOC-PHONE-SYNC") || strings.Contains(string(step), "build-helper") || strings.Contains(string(step), "requestAuthorization") {
		t.Fatal("remote step did not reuse the granted helper")
	}
}

func TestPhoneFocusFailsWhenAirUnavailable(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("PATH", dir)
	if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte("#!/bin/sh\nexit 1\n"), 0700); err != nil {
		t.Fatal(err)
	}
	if release, err := lockAirFocus(context.Background()); err == nil {
		release()
		t.Fatal("accepted missing Air lock")
	}
}
