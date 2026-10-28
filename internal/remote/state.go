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
	ID                string    `json:"id"`
	State             string    `json:"state"`
	Backend           string    `json:"backend"`
	URL               string    `json:"url"`
	LastError         string    `json:"last_error"`
	Message           string    `json:"message,omitempty"`
	PendingLink       string    `json:"pending_link"`
	TLSTermination    string    `json:"tls_termination"`
	CertificateExpiry time.Time `json:"certificate_expiry"`
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
	return CommandWithOpener(ctx, home, action, via, name, out, openLink)
}

func CommandWithOpener(ctx context.Context, home, action, via, name string, out io.Writer, opener func(context.Context, string) error) error {
	if via == "tailscale-app" {
		via = "system"
	}
	if action == "status" {
		var status Status
		if err := read(home, "status", &status); errors.Is(err, os.ErrNotExist) {
			status.State = "off"
			status.Backend = "embedded"
			status.TLSTermination = "on this Mac"
		} else if err != nil {
			return err
		}
		return json.NewEncoder(out).Encode(status)
	}
	if action != "on" && action != "off" || via != "" && via != "system" && via != "embedded" {
		return contracts.ErrInvalid
	}
	if via == "" && action == "on" {
		via = "embedded"
	}
	cfg := Config{ID: contracts.NewID("remote"), Enabled: action == "on", Backend: via, Name: name}
	if !cfg.Enabled && cfg.Backend == "" {
		var previous Status
		if read(home, "status", &previous) == nil {
			cfg.Backend = previous.Backend
		}
	}
	if cfg.Backend == "" {
		cfg.Backend = "embedded"
	}
	if cfg.Name == "" {
		cfg.Name = "bridge"
	}
	listener, err := controlListener(home, cfg.ID)
	if err != nil {
		return err
	}
	defer listener.Close()
	links := make(chan string)
	finished := make(chan struct{})
	defer close(finished)
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		go func() { <-finished; conn.Close() }()
		decoder := json.NewDecoder(conn)
		for {
			var link string
			if decoder.Decode(&link) != nil {
				return
			}
			select {
			case links <- link:
			case <-finished:
				return
			}
		}
	}()
	if err := configure(home, cfg, ""); err != nil {
		return err
	}
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	notice := ""
	opened := map[string]bool{}
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
					fmt.Fprintln(out, status.URL+"/mcp")
				} else {
					fmt.Fprintln(out, status.State)
				}
				return nil
			}
			if status.State == "error" {
				return fmt.Errorf("remote: %s", status.LastError)
			}
		}
		var current Config
		if read(home, "config", &current) == nil && current.ID != cfg.ID {
			return fmt.Errorf("remote request superseded")
		}
		select {
		case link := <-links:
			if !opened[link] {
				fmt.Fprintln(out, link)
				if opener(ctx, link) != nil {
					fmt.Fprintln(out, "Open the link above in your browser")
				}
				opened[link] = true
			}
		case <-ctx.Done():
			configure(home, Config{ID: cfg.ID, Backend: cfg.Backend, Name: cfg.Name}, cfg.ID)
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
