package scenarios

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type proxyTranscript struct {
	sync.Mutex
	data []byte
}

func (t *proxyTranscript) Write(p []byte) (int, error) {
	t.Lock()
	defer t.Unlock()
	t.data = append(t.data, p...)
	return len(p), nil
}
func (t *proxyTranscript) snapshot() []byte { t.Lock(); defer t.Unlock(); return bytes.Clone(t.data) }
func proxyHasFrame(data []byte, kind string) bool {
	for len(data) >= 4 {
		n := int(binary.BigEndian.Uint32(data))
		if n <= 0 || len(data) < 4+n {
			return false
		}
		var f struct {
			T string `json:"t"`
		}
		if json.Unmarshal(data[4:4+n], &f) == nil && f.T == kind {
			return true
		}
		data = data[4+n:]
	}
	return false
}
func proxyPair() (*net.UnixConn, *os.File, error) {
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		return nil, nil, err
	}
	parent := os.NewFile(uintptr(fds[0]), "parent")
	child := os.NewFile(uintptr(fds[1]), "child")
	conn, err := net.FileConn(parent)
	parent.Close()
	if err != nil {
		child.Close()
		return nil, nil, err
	}
	return conn.(*net.UnixConn), child, nil
}
func runSessionProxy(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Conditions: map[string]string{"session_impl": "rust", "topology": "Go gateway <-> ab-session <-> SimApplePeer"}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	binaryPath := os.Getenv("AB_SESSION_BINARY")
	if binaryPath == "" {
		add("AB_SESSION_BINARY_required", false)
		return
	}
	w, err := world.New(ctx, "sim", 0, 0)
	add("isolated_go_gateway", err == nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
	if err != nil {
		return
	}
	defer w.Close()
	w.StopHelper()
	var cfg world.Config
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err == nil {
		err = json.Unmarshal(raw, &cfg)
	}
	add("gateway_config", err == nil)
	if err != nil {
		return
	}
	gateway, err := net.DialUnix("unix", nil, &net.UnixAddr{Name: cfg.Socket, Net: "unix"})
	add("gateway_connected", err == nil)
	if err != nil {
		return
	}
	defer gateway.Close()
	left, childLeft, err := proxyPair()
	add("gateway_socketpair", err == nil)
	if err != nil {
		return
	}
	defer left.Close()
	defer childLeft.Close()
	right, childRight, err := proxyPair()
	add("helper_socketpair", err == nil)
	if err != nil {
		return
	}
	defer right.Close()
	defer childRight.Close()
	command := exec.CommandContext(ctx, binaryPath, "--gateway-fd", "3", "--helper-fd", "4")
	command.ExtraFiles = []*os.File{childLeft, childRight}
	command.Stderr = os.Stderr
	err = command.Start()
	add("rust_child_started", err == nil)
	if err != nil {
		return
	}
	defer func() { command.Process.Kill(); command.Wait() }()
	childLeft.Close()
	childRight.Close()
	directory, err := os.MkdirTemp("", "ab-session-sim-")
	add("sim_directory", err == nil)
	if err != nil {
		return
	}
	defer os.RemoveAll(directory)
	socket := filepath.Join(directory, "ipc.sock")
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: socket, Net: "unix"})
	add("sim_endpoint", err == nil)
	if err != nil {
		return
	}
	defer listener.Close()
	simCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	go simapple.Run(simCtx, simapple.Config{Socket: socket, SecretFile: filepath.Join(w.Gateway.Home, "apple-secret"), Control: filepath.Join(directory, "control.sock")})
	listener.SetDeadline(time.Now().Add(10 * time.Second))
	helper, err := listener.AcceptUnix()
	add("simapple_connected", err == nil)
	if err != nil {
		return
	}
	defer helper.Close()
	var sentGateway, receivedHelper, sentHelper, receivedGateway proxyTranscript
	var copies sync.WaitGroup
	copyStream := func(dst, src *net.UnixConn, transcript *proxyTranscript) {
		copies.Add(1)
		go func() { defer copies.Done(); io.Copy(io.MultiWriter(dst, transcript), src); dst.CloseWrite() }()
	}
	copyStream(left, gateway, &sentGateway)
	copyStream(helper, right, &receivedHelper)
	copyStream(right, helper, &sentHelper)
	copyStream(gateway, left, &receivedGateway)
	err = w.Await(10*time.Second, func() bool {
		a, b, c, d := sentGateway.snapshot(), receivedHelper.snapshot(), sentHelper.snapshot(), receivedGateway.snapshot()
		return len(a) > 0 && bytes.Equal(a, b) && bytes.Equal(c, d) && proxyHasFrame(a, "Committed") && proxyHasFrame(c, "Capture") && proxyHasFrame(c, "Ready")
	})
	add("authentication_ready_capture_committed_byte_identity", err == nil)
	helper.CloseWrite()
	gateway.CloseWrite()
	done := make(chan struct{})
	go func() { copies.Wait(); close(done) }()
	select {
	case <-done:
		add("bidirectional_eof_drained", true)
	case <-time.After(5 * time.Second):
		add("bidirectional_eof_drained", false)
		gateway.Close()
		helper.Close()
		left.Close()
		right.Close()
		<-done
	}
	a, b, c, d := sentGateway.snapshot(), receivedHelper.snapshot(), sentHelper.snapshot(), receivedGateway.snapshot()
	e.Metrics = map[string]any{"gateway_to_helper_bytes": len(a), "helper_to_gateway_bytes": len(c)}
	add("gateway_to_helper_exact_transcript", len(a) > 0 && bytes.Equal(a, b))
	add("helper_to_gateway_exact_transcript", len(c) > 0 && bytes.Equal(c, d))
	return
}
