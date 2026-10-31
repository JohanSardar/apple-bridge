package web

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

func connectorSocket(home string) string {
	absolute, _ := filepath.Abs(home)
	if canonical, err := filepath.EvalSymlinks(absolute); err == nil {
		absolute = canonical
	}
	hash := sha256.Sum256([]byte(absolute))
	return filepath.Join(fmt.Sprintf("/tmp/ab-connectors-%d", os.Getuid()), fmt.Sprintf("%x.sock", hash[:16]))
}

func ServeConnectorApprovals(home string, approvals *ConnectorApprovals) (func(), error) {
	path := connectorSocket(home)
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	info, err := os.Lstat(dir)
	if err != nil {
		return nil, err
	}
	if !info.IsDir() || info.Mode().Perm() != 0700 || info.Sys().(*syscall.Stat_t).Uid != uint32(os.Getuid()) {
		return nil, fmt.Errorf("Connector approval folder must be private to your Mac account")
	}
	if _, err := os.Lstat(path); err == nil {
		connection, err := net.DialTimeout("unix", path, time.Second)
		if err == nil {
			connection.Close()
			return nil, fmt.Errorf("Connector approvals are already running")
		}
		if err := os.Remove(path); err != nil {
			return nil, err
		}
	}
	listener, err := net.Listen("unix", path)
	if err != nil {
		return nil, err
	}
	if err := os.Chmod(path, 0600); err != nil {
		listener.Close()
		return nil, err
	}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /pending", func(w http.ResponseWriter, r *http.Request) { respond(w, 200, approvals.Pending()) })
	mux.HandleFunc("POST /decision", func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			ID      string `json:"id"`
			Code    string `json:"code"`
			Approve *bool  `json:"approve"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&in) != nil || in.Approve == nil || approvals.Decide(in.ID, in.Code, *in.Approve) != nil {
			reject(w, 400, "Approval refused. Check the code and start a new connection if it expired.")
			return
		}
		respond(w, 200, map[string]bool{"accepted": true})
	})
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 3 * time.Second, ReadTimeout: 5 * time.Second, WriteTimeout: 5 * time.Second, IdleTimeout: 5 * time.Second}
	go server.Serve(listener)
	return func() { server.Close() }, nil
}

func ConnectorCommand(ctx context.Context, home, action, id, code string, output io.Writer) error {
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "unix", connectorSocket(home))
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second}
	method, path := "GET", "/pending"
	var body io.Reader
	if action != "pending" {
		if action != "allow" && action != "deny" {
			return fmt.Errorf("Use connector pending, allow or deny")
		}
		raw, _ := json.Marshal(map[string]any{"id": id, "code": code, "approve": action == "allow"})
		body = bytes.NewReader(raw)
		method, path = "POST", "/decision"
	}
	request, err := http.NewRequestWithContext(ctx, method, "http://local"+path, body)
	if err != nil {
		return err
	}
	response, err := client.Do(request)
	if err != nil {
		return fmt.Errorf("Cannot reach Apple Bridge approvals. Open Apple Bridge and try again")
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return fmt.Errorf("Approval refused. Check the code and start a new connection if it expired")
	}
	_, err = io.Copy(output, io.LimitReader(response.Body, 1<<20))
	return err
}
