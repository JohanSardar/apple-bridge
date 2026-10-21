package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runHelperIPC(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	w, err := world.New(ctx, "sim", 1, 0)
	add("isolated_gateway_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	defer w.SignalGateway(syscall.SIGKILL)
	w.StopHelper()
	var cfg world.Config
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err == nil {
		err = json.Unmarshal(raw, &cfg)
	}
	if err != nil {
		add("gateway_configuration_read", false)
		return
	}
	secret, err := os.ReadFile(filepath.Join(w.Gateway.Home, "apple-secret"))
	if err != nil {
		add("helper_authentication_secret_read", false)
		return
	}
	connection, err := (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, "unix", cfg.Socket)
	if err != nil {
		add("helper_socket_connected", false)
		return
	}
	conn := connection.(*net.UnixConn)
	defer conn.Close()
	wire := &applepeer.Wire{Conn: conn}
	err = applepeer.Authenticate(wire, secret, false)
	if err != nil {
		add("helper_authenticated", false)
		return
	}
	opened, _, err := wire.Receive()
	add("helper_authenticated_with_bounded_credit", err == nil && opened.T == "Open" && opened.ByteCredit == contracts.ByteCredit)
	if err != nil {
		return
	}
	_, err = wire.Send(contracts.IPCFrame{T: "Ready", Protocol: contracts.IPCProtocol, Coverage: map[contracts.Source]contracts.Coverage{contracts.Reminders: {Fields: "complete"}}})
	if err == nil {
		_, err = wire.Send(contracts.IPCFrame{T: "Ping"})
	}
	if err == nil {
		err = helperPong(conn, wire, time.Now().Add(5*time.Second))
	}
	add("helper_ready_before_capture_hold", err == nil)
	if err != nil {
		return
	}
	captureBytes := 0
	err = os.WriteFile(filepath.Join(w.Gateway.Home, "lab-capture-armed"), []byte("armed"), 0600)
	if err == nil {
		captureBytes, err = wire.Send(contracts.IPCFrame{T: "Capture", Capture: &contracts.Capture{Source: contracts.Reminders, Generation: contracts.NewID("generation"), Checkpoint: contracts.NewID("checkpoint"), Live: true}})
	}
	if err == nil {
		err = w.Await(3*time.Second, func() bool {
			_, err := os.Stat(filepath.Join(w.Gateway.Home, "lab-capture-reached"))
			return err == nil
		})
	}
	add("capture_commit_held_before_acknowledgment", err == nil)
	if err != nil {
		return
	}
	payload := bytes.Repeat([]byte{1}, 128<<10)
	digest := sha256.Sum256(payload)
	conn.SetWriteBuffer(4096)
	conn.SetWriteDeadline(time.Now().Add(time.Second))
	started := time.Now()
	_, err = contracts.WriteFrame(conn, contracts.IPCFrame{T: "BlobChunk", Handle: "ipc-probe", Digest: hex.EncodeToString(digest[:]), Bytes: payload, EOF: true})
	elapsed := float64(time.Since(started)) / float64(time.Millisecond)
	timeouts := 0
	if err != nil {
		timeouts = 1
	}
	e.Latency["ipc_blob_reply_write"] = stats([]float64{elapsed}, timeouts)
	add("blob_reply_progresses_while_capture_commit_is_held", err == nil && elapsed < 1000)
	if err != nil {
		return
	}
	started = time.Now()
	_, err = wire.Send(contracts.IPCFrame{T: "Ping"})
	if err == nil {
		err = helperPong(conn, wire, time.Now().Add(time.Second))
	}
	elapsed = float64(time.Since(started)) / float64(time.Millisecond)
	timeouts = 0
	if err != nil {
		timeouts = 1
	}
	e.Latency["ipc_control_round_trip"] = stats([]float64{elapsed}, timeouts)
	add("control_progresses_without_early_capture_ack", err == nil && elapsed < 1000)
	if err == nil {
		add("capture_credit_exhaustion_closes_session", helperCreditExhausted(conn, wire, captureBytes))
	}
	return
}

func helperPong(conn *net.UnixConn, wire *applepeer.Wire, deadline time.Time) error {
	conn.SetReadDeadline(deadline)
	for {
		var frame contracts.IPCFrame
		if _, err := contracts.ReadFrame(conn, &frame); err != nil {
			return err
		}
		if frame.T == "Pong" {
			return nil
		}
		if frame.T != "Ping" {
			return fmt.Errorf("unexpected IPC frame %s before Pong", frame.T)
		}
		if _, err := wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
			return err
		}
	}
}

func helperCreditExhausted(conn *net.UnixConn, wire *applepeer.Wire, outstanding int) bool {
	frame := contracts.IPCFrame{T: "Capture", Capture: &contracts.Capture{Source: contracts.Reminders, Generation: "probe", Checkpoint: "queued", Observations: []contracts.Observation{{Kind: "upsert", Collection: "reminders.reminder", NativeKey: "probe", Data: json.RawMessage(`{"title":"` + strings.Repeat("x", 64<<10) + `"}`)}}}}
	encoded, err := contracts.EncodeFrame(frame)
	if err != nil {
		return false
	}
	for outstanding+len(encoded) <= contracts.ByteCredit {
		n, err := wire.Send(frame)
		if err != nil {
			return false
		}
		outstanding += n
	}
	_, err = wire.Send(frame)
	conn.SetReadDeadline(time.Now().Add(time.Second))
	for err == nil {
		var reply contracts.IPCFrame
		_, err = contracts.ReadFrame(conn, &reply)
		if err == nil {
			if reply.T != "Ping" {
				return false
			}
			_, err = wire.Send(contracts.IPCFrame{T: "Pong"})
		}
	}
	var timeout net.Error
	return err != nil && (!errors.As(err, &timeout) || !timeout.Timeout())
}
