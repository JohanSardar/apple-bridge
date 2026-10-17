package runtime

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type CredentialStoreReport struct {
	Store     string                      `json:"store"`
	LastError *trust.CredentialStoreError `json:"last_error,omitempty"`
}

type DoctorReport struct {
	Credentials CredentialStoreReport `json:"credentials"`
	state.Integrity
	Warnings           []string          `json:"warnings,omitempty"`
	ClockOffsetMS      *int64            `json:"clock_offset_ms,omitempty"`
	ClockUncertaintyMS *int64            `json:"clock_uncertainty_ms,omitempty"`
	ClockRTTMS         *int64            `json:"clock_rtt_ms,omitempty"`
	ClockSkewMS        *int64            `json:"clock_skew_ms,omitempty"`
	ClockSampleAgeMS   *int64            `json:"clock_sample_age_ms,omitempty"`
	ClockMethod        string            `json:"clock_method,omitempty"`
	PrivateModes       bool              `json:"private_modes"`
	Encryption         string            `json:"disk_encryption"`
	ClockSkew          string            `json:"clock_skew"`
	Versions           map[string]string `json:"versions"`
}

func Doctor(ctx context.Context, home string) (DoctorReport, error) {
	credentials := CredentialStoreReport{Store: trust.NewSecretStore(home).Backend()}
	if data, err := os.ReadFile(filepath.Join(home, "credential-store-error.json")); err == nil {
		var failure trust.CredentialStoreError
		if json.Unmarshal(data, &failure) == nil && failure.Store == credentials.Store {
			credentials.LastError = &failure
		}
	}
	store, err := state.InspectHome(home)
	if err != nil {
		if credentials.LastError != nil {
			return DoctorReport{Credentials: credentials}, nil
		}
		return DoctorReport{}, err
	}
	defer store.Close()
	integrity, err := store.Integrity(ctx)
	if err != nil {
		if credentials.LastError != nil {
			return DoctorReport{Credentials: credentials}, nil
		}
		return DoctorReport{}, err
	}
	report := DoctorReport{Credentials: credentials, Integrity: integrity, PrivateModes: true, Encryption: "unverified", ClockSkew: "unverified", Versions: map[string]string{"runtime": goruntime.Version(), "ipc": contracts.IPCProtocol, "feed": contracts.FeedProtocol}}
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
	if goruntime.GOOS == "linux" {
		report.Encryption = linuxEncryption(ctx, home)
	}
	if report.Encryption == "unverified" {
		report.Warnings = append(report.Warnings, "Disk encryption could not be checked; install findmnt/lsblk on Linux or check FileVault status on macOS, and verify encryption for the data directory.")
	}
	if report.Encryption == "warning_not_enabled" {
		report.Warnings = append(report.Warnings, "The data directory is not on detected encrypted storage; enable FileVault or move the directory onto LUKS/dm-crypt storage.")
	}
	cfg, configErr := LoadConfig(home)
	if configErr == nil && cfg.Role == "gateway" {
		report.ClockSkew = "n/a"
	} else {
		raw, readErr := os.ReadFile(filepath.Join(home, "gateway-clock.json"))
		var sample contracts.ClockSample
		decodeErr := json.Unmarshal(raw, &sample)
		offset, uncertainty, estimateErr := sample.Estimate()
		if readErr == nil && decodeErr == nil && estimateErr == nil {
			skew, age := offset.Milliseconds(), time.Since(sample.T3).Milliseconds()
			uncertaintyMS := int64((uncertainty + time.Millisecond - 1) / time.Millisecond)
			rttMS := int64(2 * uncertainty / time.Millisecond)
			report.ClockOffsetMS, report.ClockUncertaintyMS, report.ClockRTTMS = &skew, &uncertaintyMS, &rttMS
			report.ClockSkewMS, report.ClockSampleAgeMS = &skew, &age
			report.ClockMethod = "authenticated_feed_ntp_four_timestamps"
			report.ClockSkew = "ok"
			if age < 0 || age > 15000 {
				report.ClockSkew = "warning_stale_sample"
				report.Warnings = append(report.Warnings, "Gateway time sample is stale; reconnect the replica and rerun doctor.")
			}
			if uncertaintyMS > 1000 && report.ClockSkew == "ok" {
				report.ClockSkew = "warning_clock_uncertainty"
				report.Warnings = append(report.Warnings, "Clock uncertainty exceeds one second; reduce round-trip network delay for a tighter offset estimate.")
			}
			if skew-uncertaintyMS > 1000 || skew+uncertaintyMS < -1000 {
				report.ClockSkew = "warning_clock_skew"
				report.Warnings = append(report.Warnings, "Clock offset exceeds one second after accounting for round-trip uncertainty; enable network time synchronization on both hosts.")
			}
		} else {
			report.Warnings = append(report.Warnings, "Authenticated gateway clock exchange unavailable; reconnect peers with clock-exchange support, then rerun doctor.")
		}
	}
	report.OK = report.OK && report.PrivateModes && credentials.LastError == nil
	return report, nil
}
