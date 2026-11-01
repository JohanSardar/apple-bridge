package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func idleSessionPath() (string, error) {
	ticket := os.Getenv("AB_TICKET")
	if ticket == "" || strings.ContainsAny(ticket, "/\\.") {
		return "", fmt.Errorf("idle setup requires a runner ticket")
	}
	return filepath.Join("/tmp/ablab", "q2-idle-"+ticket+".json"), nil
}

func idleSetup(ctx context.Context, id string, asJSON bool) error {
	path, err := idleSessionPath()
	if err != nil {
		return err
	}
	sha, err := exec.Command("git", "rev-parse", "HEAD").Output()
	if err != nil {
		return err
	}
	revision := strings.TrimSpace(string(sha))
	e := Evidence{RunID: contracts.NewID("run"), Scenario: id, Level: "real", GitSHA: revision, At: time.Now().UTC()}
	var w *world.World
	if id == "Q2-IDLE-PREP" {
		if _, err = os.Lstat(path); !os.IsNotExist(err) {
			return fmt.Errorf("idle setup session already exists")
		}
		w, err = world.NewReal(ctx, "real", 0, world.BlobOptions{Scope: "admin"})
		if err == nil {
			err = w.SaveSession(path, revision)
			if err == nil {
				timer := time.NewTimer(3 * time.Minute)
				select {
				case <-ctx.Done():
					err = ctx.Err()
				case <-timer.C:
				}
				timer.Stop()
			}
		}
	} else {
		w, err = world.ResumeSession(ctx, path, revision)
		if err == nil && !awaitIdleCapture(ctx, w, &e) {
			err = fmt.Errorf("idle capture did not settle")
		}
	}
	if err != nil && w != nil {
		w.Close()
		os.Remove(path)
	}
	e.Passed = err == nil
	e.Assertions = append(e.Assertions, Assertion{Name: "same_isolated_helper_prepared", Passed: e.Passed})
	if saveErr := Save(e); saveErr != nil {
		if w != nil {
			w.Close()
		}
		os.Remove(path)
		return saveErr
	}
	if asJSON {
		json.NewEncoder(os.Stdout).Encode(e)
	} else {
		fmt.Printf("%s real passed=%t\n", id, e.Passed)
	}
	return err
}
