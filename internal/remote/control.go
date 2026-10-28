package remote

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

func controlPath(home, id string) string {
	if absolute, err := filepath.Abs(home); err == nil {
		home = absolute
	}
	if canonical, err := filepath.EvalSymlinks(home); err == nil {
		home = canonical
	}
	sum := sha256.Sum256([]byte(filepath.Clean(home) + "/" + id))
	return filepath.Join(fmt.Sprintf("/tmp/ab-remote-%d", os.Getuid()), fmt.Sprintf("%x.sock", sum[:16]))
}

func configure(home string, cfg Config, matching string) error {
	dir := filepath.Join(home, "remote")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	lock, err := os.OpenFile(filepath.Join(dir, "config.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return err
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	if matching != "" {
		var current Config
		if read(home, "config", &current) != nil || current.ID != matching {
			return nil
		}
	}
	return write(home, "config", cfg)
}

func (r *Remote) approval(status *Status, link string) {
	if link == "" {
		return
	}
	r.mu.RLock()
	defer r.mu.RUnlock()
	if r.control != nil && r.controlID == status.ID {
		r.control.SetWriteDeadline(time.Now().Add(time.Second))
		json.NewEncoder(r.control).Encode(link)
	}
}

func (r *Remote) cancelled(cfg Config) {
	cfg.Enabled = false
	configure(r.home, cfg, cfg.ID)
	var current Config
	if read(r.home, "config", &current) == nil && current.ID == cfg.ID && !current.Enabled {
		r.publish(Status{ID: cfg.ID, State: "off", Backend: cfg.Backend, TLSTermination: "on this Mac"})
	}
}

func controlListener(home, id string) (net.Listener, error) {
	if err := os.MkdirAll(filepath.Join(home, "remote"), 0700); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(filepath.Dir(controlPath(home, id)), 0700); err != nil {
		return nil, err
	}
	path := controlPath(home, id)
	info, err := os.Lstat(filepath.Dir(path))
	if err != nil {
		return nil, err
	}
	if info.Mode()&os.ModeSymlink != 0 || info.Mode().Perm() != 0700 || info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) {
		return nil, fmt.Errorf("approval IPC directory must be private")
	}
	listener, err := net.Listen("unix", path)
	if err != nil {
		return nil, err
	}
	if err := os.Chmod(path, 0600); err != nil {
		listener.Close()
		return nil, err
	}
	return listener, nil
}
