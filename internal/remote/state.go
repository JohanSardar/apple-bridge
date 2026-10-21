package remote

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Config struct {
	ID      string `json:"id"`
	Enabled bool   `json:"enabled"`
	Backend string `json:"backend,omitempty"`
	Name    string `json:"name,omitempty"`
}

type Status struct {
	ID        string `json:"id"`
	State     string `json:"state"`
	Backend   string `json:"backend"`
	URL       string `json:"url"`
	LastError string `json:"last_error"`
	Message   string `json:"message,omitempty"`
}

func read(home, name string, value any) error {
	data, err := os.ReadFile(filepath.Join(home, "remote", name+".json"))
	if err != nil {
		return err
	}
	return json.Unmarshal(data, value)
}

func write(home, name string, value any) error {
	dir := filepath.Join(home, "remote")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	if err := os.Chmod(dir, 0700); err != nil {
		return err
	}
	data, err := json.Marshal(value)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".remote-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if err = tmp.Chmod(0600); err == nil {
		_, err = tmp.Write(data)
	}
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(tmp.Name(), filepath.Join(dir, name+".json"))
}

func Command(ctx context.Context, home, action, via, name string, out io.Writer) error {
	if action == "status" {
		var status Status
		if err := read(home, "status", &status); errors.Is(err, os.ErrNotExist) {
			status.State = "off"
		} else if err != nil {
			return err
		}
		return json.NewEncoder(out).Encode(status)
	}
	if action != "on" && action != "off" || via != "" && via != "system" && via != "embedded" {
		return contracts.ErrInvalid
	}
	cfg := Config{ID: contracts.NewID("remote"), Enabled: action == "on", Backend: via, Name: name}
	if !cfg.Enabled && cfg.Backend == "" {
		var previous Status
		if read(home, "status", &previous) == nil {
			cfg.Backend = previous.Backend
		}
	}
	if cfg.Name == "" {
		cfg.Name = "bridge"
	}
	if err := write(home, "config", cfg); err != nil {
		return err
	}
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	notice := ""
	for {
		var status Status
		if read(home, "status", &status) == nil && status.ID == cfg.ID {
			if status.Message != "" && status.Message != notice {
				fmt.Fprintln(out, status.Message)
				notice = status.Message
			}
			if status.State == "on" || status.State == "off" {
				if status.URL != "" {
					fmt.Fprintln(out, status.URL)
				} else {
					fmt.Fprintln(out, status.State)
				}
				return nil
			}
			if status.State == "error" {
				return fmt.Errorf("remote: %s", status.LastError)
			}
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
