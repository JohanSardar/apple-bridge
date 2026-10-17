package world

import (
	"encoding/json"
	"path/filepath"
	"time"
)

func (w *World) DoctorClockWarnings(endpoint Endpoint) (bool, bool, error) {
	path := filepath.Join(endpoint.Home, "gateway-clock.json")
	saved, err := w.execute(endpoint.Remote, "cat", path)
	if err != nil {
		return false, false, err
	}
	if err = w.PauseReplica(0); err != nil {
		return false, false, err
	}
	defer w.ResumeReplica(0)
	write := func(raw string) error {
		_, err := w.execute(endpoint.Remote, "sh", "-c", "printf %s "+quote(raw)+" > "+quote(path))
		return err
	}
	defer write(saved)
	now := time.Now().UTC()
	raw, _ := json.Marshal(map[string]time.Time{"gateway": now.Add(-10 * time.Second), "received": now})
	if err = write(string(raw)); err != nil {
		return false, false, err
	}
	skew, err := w.Doctor(endpoint)
	if err != nil {
		return false, false, err
	}
	if err = write("{}"); err != nil {
		return false, false, err
	}
	missing, err := w.Doctor(endpoint)
	skewWarnings, _ := skew["warnings"].([]any)
	missingWarnings, _ := missing["warnings"].([]any)
	return skew["clock_skew"] == "warning_clock_skew" && len(skewWarnings) > 0, missing["clock_skew"] == "unverified" && len(missingWarnings) > 0, err
}
