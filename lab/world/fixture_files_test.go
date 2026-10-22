package world

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestFixtureExecutableSurvivesCacheRemoval(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	if os.Getenv("APPLE_BRIDGE_FIXTURE_HASH_CHILD") == "1" {
		if filepath.Base(executable) != "fixture-hash-test" {
			t.Fatal("owned test executable required")
		}
		gateway := filepath.Join(t.TempDir(), "gateway")
		if err := os.WriteFile(gateway, []byte("gateway-one"), 0600); err != nil {
			t.Fatal(err)
		}
		before, err := fixtureFingerprint("inputs", gateway)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.Remove(executable); err != nil {
			t.Fatal(err)
		}
		after, err := fixtureFingerprint("inputs", gateway)
		if err != nil || after != before {
			t.Fatalf("fingerprint after executable removal: %q %v", after, err)
		}
		if err := os.WriteFile(gateway, []byte("gateway-two"), 0600); err != nil {
			t.Fatal(err)
		}
		changed, err := fixtureFingerprint("inputs", gateway)
		if err != nil || changed == before {
			t.Fatalf("changed gateway must invalidate: %q %v", changed, err)
		}
		staged := filepath.Join(t.TempDir(), "staged-lab-test")
		if err := stageExecutable(staged); err != nil {
			t.Fatal(err)
		}
		digest, err := fixtureFileDigest(staged)
		if err != nil || digest != labImage.digest {
			t.Fatalf("staged executable digest: %q %v", digest, err)
		}
		probe := exec.Command(staged, "-test.list=^TestFixtureExecutableSurvivesCacheRemoval$")
		if output, err := probe.CombinedOutput(); err != nil {
			t.Fatalf("staged executable launch: %v %s", err, output)
		}
		previous := changed
		changed, err = fixtureFingerprint("changed-inputs", gateway)
		if err != nil || changed == previous {
			t.Fatalf("changed inputs must invalidate: %q %v", changed, err)
		}
		return
	}
	body, err := os.ReadFile(executable)
	if err != nil {
		t.Fatal(err)
	}
	copied := filepath.Join(t.TempDir(), "fixture-hash-test")
	if err := os.WriteFile(copied, body, 0700); err != nil {
		t.Fatal(err)
	}
	command := exec.Command(copied, "-test.run=^TestFixtureExecutableSurvivesCacheRemoval$", "-test.v")
	command.Env = append(os.Environ(), "APPLE_BRIDGE_FIXTURE_HASH_CHILD=1")
	output, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("owned executable removal proof: %v\n%s", err, output)
	}
}
