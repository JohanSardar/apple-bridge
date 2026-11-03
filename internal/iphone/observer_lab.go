//go:build lab

package iphone

import (
	"bufio"
	"bytes"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"os"
	"time"
)

type observation struct {
	Event    string `json:"event"`
	Path     string `json:"path,omitempty"`
	DeviceID string `json:"device_id,omitempty"`
	At       int64  `json:"at,omitempty"`
	Payload  []byte `json:"payload,omitempty"`
	Status   int    `json:"status,omitempty"`
	Body     []byte `json:"body,omitempty"`
	Request  int64  `json:"request,omitempty"`
}

type observedWriter struct {
	http.ResponseWriter
	status int
	body   bytes.Buffer
}

func (w *observedWriter) WriteHeader(status int) {
	w.status = status
	w.ResponseWriter.WriteHeader(status)
}

func (w *observedWriter) Write(raw []byte) (int, error) {
	if w.status == 0 {
		w.status = http.StatusOK
	}
	w.body.Write(raw)
	return w.ResponseWriter.Write(raw)
}

func send(path string, value observation) (int64, error) {
	conn, err := net.DialTimeout("unix", path, 5*time.Second)
	if err != nil {
		return 0, err
	}
	defer conn.Close()
	conn.SetDeadline(time.Now().Add(30 * time.Second))
	raw, err := json.Marshal(value)
	if err != nil {
		return 0, err
	}
	if _, err = conn.Write(append(raw, '\n')); err != nil {
		return 0, err
	}
	line, err := bufio.NewReader(conn).ReadBytes('\n')
	if err != nil {
		return 0, err
	}
	var ack struct {
		Sequence int64 `json:"sequence"`
	}
	if err = json.Unmarshal(line, &ack); err != nil {
		return 0, err
	}
	return ack.Sequence, nil
}

func observe(next http.Handler) http.Handler {
	socket := os.Getenv("APPLE_BRIDGE_LAB_IPHONE_OBSERVER")
	if socket == "" {
		return next
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		raw, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 25<<20))
		if err != nil {
			http.Error(w, "invalid", 400)
			return
		}
		r.Body = io.NopCloser(bytes.NewReader(raw))
		var request struct {
			DeviceID string `json:"device_id"`
			At       int64  `json:"at"`
			Payload  []byte `json:"payload"`
		}
		if r.Method != http.MethodPost || json.Unmarshal(raw, &request) != nil {
			next.ServeHTTP(w, r)
			return
		}
		sequence, err := send(socket, observation{Event: "request", Path: r.URL.Path, DeviceID: request.DeviceID, At: request.At, Payload: request.Payload})
		if err != nil {
			http.Error(w, "observer unavailable", 503)
			return
		}
		recorder := &observedWriter{ResponseWriter: w}
		next.ServeHTTP(recorder, r)
		if recorder.status == 0 {
			recorder.status = http.StatusOK
		}
		send(socket, observation{Event: "response", Request: sequence, Status: recorder.status, Body: recorder.body.Bytes()})
	})
}
