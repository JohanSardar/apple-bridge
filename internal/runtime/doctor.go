package runtime

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	goruntime "runtime"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

type DoctorReport struct {
	state.Integrity
	PrivateModes bool              `json:"private_modes"`
	Encryption   string            `json:"disk_encryption"`
	ClockSkew    string            `json:"clock_skew"`
	Versions     map[string]string `json:"versions"`
}

func Doctor(ctx context.Context, home string) (DoctorReport, error) {
	store, err := state.InspectHome(home)
	if err != nil {
		return DoctorReport{}, err
	}
	defer store.Close()
	integrity, err := store.Integrity(ctx)
	if err != nil {
		return DoctorReport{}, err
	}
	report := DoctorReport{Integrity: integrity, PrivateModes: true, Encryption: "unverified", ClockSkew: "unverified", Versions: map[string]string{"runtime": goruntime.Version(), "ipc": contracts.IPCProtocol, "feed": contracts.FeedProtocol}}
	err = filepath.WalkDir(home, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		if info.Mode()&os.ModeSymlink != 0 {
			report.PrivateModes = false
			return nil
		}
		expected := os.FileMode(0600)
		if info.IsDir() {
			expected = 0700
		}
		if info.Mode().Perm() != expected {
			report.PrivateModes = false
		}
		return nil
	})
	if err != nil {
		return report, err
	}
	if goruntime.GOOS == "darwin" {
		output, err := exec.CommandContext(ctx, "fdesetup", "status").Output()
		if err == nil {
			if strings.Contains(string(output), "FileVault is On") {
				report.Encryption = "enabled"
			} else {
				report.Encryption = "warning_not_enabled"
			}
		}
	}
	report.OK = report.OK && report.PrivateModes
	return report, nil
}
