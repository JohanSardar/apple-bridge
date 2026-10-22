package scenarios

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runNativeMediaPosters(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{"synthetic_only": true}}
	directory, err := os.MkdirTemp(filepath.Join("lab", "evidence", "private"), "ab-test-poster-")
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "synthetic_fixture", Passed: false})
		return e
	}
	defer os.RemoveAll(directory)
	tag := []byte(e.RunID)
	box := make([]byte, 8+len(tag))
	binary.BigEndian.PutUint32(box, uint32(len(box)))
	copy(box[4:], "free")
	copy(box[8:], tag)
	source := filepath.Join(directory, "panorama.mp4")
	err = os.WriteFile(source, append(simapple.MediaFixture("video-panorama"), box...), 0600)
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "synthetic_fixture", Passed: false})
		return e
	}
	var diagnostics bytes.Buffer
	command := exec.CommandContext(ctx, mediaTool(), "poster-safety", source)
	command.Stderr = &diagnostics
	output, err := command.Output()
	log := filepath.Join("lab", "evidence", "private", e.RunID+"-poster-safety.log")
	os.WriteFile(log, append(output, diagnostics.Bytes()...), 0600)
	metrics := map[string]any{}
	decoded := json.Unmarshal(output, &metrics) == nil
	e.Assertions = append(e.Assertions, Assertion{Name: "native_poster_resolution_sharpness_cache_and_live_still", Passed: err == nil && decoded})
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = "synthetic native poster check: " + err.Error()
	}
	if decoded {
		e.Metrics["poster"] = metrics
	}
	e.Passed = err == nil && decoded
	return e
}
