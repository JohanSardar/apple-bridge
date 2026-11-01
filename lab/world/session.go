package world

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

type sessionProcess struct {
	PID      int
	Identity string
}

type savedSession struct {
	Directory string
	HelperApp string
	Gateway   Endpoint
	Processes []sessionProcess
	Helper    sessionProcess
	Created   time.Time
	Revision  string
}

func processIdentity(pid int) (string, error) {
	data, err := exec.Command("ps", "-p", strconv.Itoa(pid), "-o", "lstart=", "-o", "command=").Output()
	return strings.TrimSpace(string(data)), err
}

func (w *World) SaveSession(path, revision string) error {
	session := savedSession{Directory: w.Dir, HelperApp: w.HelperApp, Gateway: w.Gateway, Created: time.Now(), Revision: revision}
	if w.Native {
		data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
		if err != nil {
			return err
		}
		pid, err := strconv.Atoi(strings.TrimSpace(string(data)))
		if err != nil {
			return err
		}
		identity, err := processIdentity(pid)
		if err != nil {
			return err
		}
		session.Helper = sessionProcess{PID: pid, Identity: identity}
	}
	for _, process := range w.processes {
		if process == nil || process.stopped {
			continue
		}
		pid := process.command.Process.Pid
		identity, err := processIdentity(pid)
		if err != nil {
			return err
		}
		session.Processes = append(session.Processes, sessionProcess{PID: pid, Identity: identity})
	}
	if len(session.Processes) == 0 {
		return fmt.Errorf("idle session has no gateway")
	}
	data, err := json.Marshal(session)
	if err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	_, err = file.Write(data)
	closeErr := file.Close()
	if err != nil {
		os.Remove(path)
		return err
	}
	return closeErr
}

func ResumeSession(ctx context.Context, path, revision string) (*World, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		return nil, fmt.Errorf("idle session permissions invalid")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var session savedSession
	if err = json.Unmarshal(data, &session); err != nil {
		return nil, err
	}
	if session.Revision != revision || time.Since(session.Created) > 2*time.Hour || time.Until(session.Created) > time.Minute || len(session.Processes) == 0 || filepath.Dir(session.Directory) != "/tmp/ablab" || session.Gateway.Home != filepath.Join(session.Directory, "gateway") || session.HelperApp != filepath.Join(session.Directory, "apps", "AppleBridgeHelper.app") {
		return nil, fmt.Errorf("idle session does not match this run")
	}
	for _, process := range session.Processes {
		identity, err := processIdentity(process.PID)
		if err != nil || identity != process.Identity {
			return nil, fmt.Errorf("idle gateway changed since setup")
		}
	}
	if session.Helper.PID != 0 {
		identity, err := processIdentity(session.Helper.PID)
		if err != nil || identity != session.Helper.Identity {
			return nil, fmt.Errorf("idle helper changed since setup")
		}
	}
	w := &World{Dir: session.Directory, HelperApp: session.HelperApp, Gateway: session.Gateway, Level: "real", Native: true, ctx: ctx}
	w.previousSecretStore, w.previousSecretStoreSet = os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
	w.secretOverride = true
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	w.nativeCleanup = func(*World) error {
		defer os.Remove(path)
		for _, process := range session.Processes {
			if identity, err := processIdentity(process.PID); err == nil && identity == process.Identity {
				syscall.Kill(process.PID, syscall.SIGTERM)
			}
		}
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			alive := false
			for _, process := range session.Processes {
				if identity, err := processIdentity(process.PID); err == nil && identity == process.Identity {
					alive = true
				}
			}
			if !alive {
				return nil
			}
			time.Sleep(100 * time.Millisecond)
		}
		for _, process := range session.Processes {
			if identity, err := processIdentity(process.PID); err == nil && identity == process.Identity {
				syscall.Kill(process.PID, syscall.SIGKILL)
			}
		}
		return nil
	}
	return w, nil
}
