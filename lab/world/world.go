package world

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Config struct {
	Role              string `json:"role"`
	Home              string `json:"home"`
	PeerAddress       string `json:"peer_address"`
	APIAddress        string `json:"api_address"`
	Socket            string `json:"socket"`
	HelperApp         string `json:"helper_app"`
	HelperLabCommands bool   `json:"helper_lab_commands"`
}
type Process struct {
	command *exec.Cmd
	done    chan error
	log     *os.File
	remote  bool
	home    string
	stopped bool
}
type Endpoint struct {
	URL    string
	Host   string
	Token  string
	Home   string
	Remote bool
}
type BlobOptions struct {
	Cap     int64
	GraceMS int64
	Corrupt bool
	Slow    bool
}
type NativeSetup struct {
	HoldBackfill bool
	BeforeStart  func(*World) error
	Cleanup      func(*World) error
}

type World struct {
	remoteExtraHomes []string

	blobOptions            BlobOptions
	secretOverride         bool
	previousSecretStore    string
	previousSecretStoreSet bool
	nativeCleanup          func(*World) error

	Dir              string
	TruthFailures    []string
	HelperApp        string
	TruthApp         string
	Native           bool
	Level            string
	Gateway          Endpoint
	Replicas         []Endpoint
	Control          *http.Client
	processes        []*Process
	replicaProcesses []*Process
	bridge           string
	lab              string
	remoteBinary     string
	remoteLab        string
	diskMount        string
	secretHomes      []string
	proxy            *FaultProxy
	NetworkPath      string
	pairAddress      string
	ctx              context.Context
}

func commandOutput(ctx context.Context, name string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, name, args...)
	var out bytes.Buffer
	cmd.Stdout = &out
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	err := cmd.Run()
	if err != nil {
		return "", fmt.Errorf("%s failed: %w: %s", name, err, strings.TrimSpace(stderr.String()))
	}
	return strings.TrimSpace(out.String()), nil
}
func Build(ctx context.Context, level string) error {
	if err := os.MkdirAll("bin", 0700); err != nil {
		return err
	}
	if _, err := commandOutput(ctx, "go", "build", "-tags", "lab", "-p", "2", "-o", "bin/bridge", "./cmd/bridge"); err != nil {
		return err
	}
	if level == "replica" {
		cmd := exec.CommandContext(ctx, "go", "build", "-tags", "lab", "-p", "2", "-o", "bin/bridge-linux-amd64", "./cmd/bridge")
		cmd.Env = append(os.Environ(), "GOOS=linux", "GOARCH=amd64")
		var log bytes.Buffer
		cmd.Stdout = &log
		cmd.Stderr = &log
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("cross compile: %w: %s", err, log.String())
		}

		lab := exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", "bin/bridge-lab-linux-amd64", "./cmd/bridge-lab")
		lab.Env = append(os.Environ(), "GOOS=linux", "GOARCH=amd64")
		output, err := lab.CombinedOutput()
		if err != nil {
			return fmt.Errorf("cross compile lab: %w: %s", err, output)
		}
	}
	return nil
}
func (w *World) start(name string, remote bool, home string, binary string, args ...string) (*Process, error) {
	file, err := os.OpenFile(filepath.Join(w.Dir, name+".log"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0600)
	if err != nil {
		return nil, err
	}
	cmd := exec.Command(binary, args...)
	if name == "gateway" {
		cmd.Env = os.Environ()
		if w.blobOptions.Corrupt {
			cmd.Env = append(cmd.Env, "APPLE_BRIDGE_LAB_BLOB_CORRUPT="+filepath.Join(w.Dir, "corrupt-once"))
		}
		if w.blobOptions.Slow {
			cmd.Env = append(cmd.Env, "APPLE_BRIDGE_LAB_BLOB_SLOW=1", "APPLE_BRIDGE_LAB_BLOB_CUT_WINDOW="+filepath.Join(w.Dir, "blob-cut-window"))
		}
	}
	if remote {
		joined := quote(binary)
		for _, arg := range args {
			joined += " " + quote(arg)
		}
		script := "umask 077; echo $$ > " + quote(filepath.Join(home, "pid")) + "; exec " + joined
		cmd = exec.Command("ssh", "ubuntu", script)
	}
	cmd.Stdout = file
	cmd.Stderr = file
	if err = cmd.Start(); err != nil {
		file.Close()
		return nil, err
	}
	p := &Process{command: cmd, done: make(chan error, 1), log: file, remote: remote, home: home}
	go func() { p.done <- cmd.Wait(); file.Close() }()
	w.processes = append(w.processes, p)
	return p, nil
}
func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", "'\"'\"'") + "'" }
func (p *Process) Stop() {
	if p == nil || p.stopped {
		return
	}
	p.stopped = true
	if p.remote {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		commandOutput(ctx, "ssh", "ubuntu", "if test -f "+quote(filepath.Join(p.home, "pid"))+"; then kill -CONT $(cat "+quote(filepath.Join(p.home, "pid"))+"); kill -TERM $(cat "+quote(filepath.Join(p.home, "pid"))+"); fi")
		cancel()
	} else {
		p.command.Process.Signal(syscall.SIGCONT)
		p.command.Process.Signal(syscall.SIGTERM)
	}
	timer := time.NewTimer(5 * time.Second)
	defer timer.Stop()
	select {
	case <-p.done:
		return
	case <-timer.C:
		if p.remote {
			p.Signal(syscall.SIGKILL)
		}
		p.command.Process.Kill()
		<-p.done
	}
}
func (w *World) execute(remote bool, binary string, args ...string) (string, error) {
	if remote {
		cmd := quote(binary)
		for _, a := range args {
			cmd += " " + quote(a)
		}
		return commandOutput(w.ctx, "ssh", "ubuntu", cmd)
	}
	return commandOutput(w.ctx, binary, args...)
}
func (w *World) config(home string, remote bool) (Config, error) {
	var cfg Config
	err := w.Await(20*time.Second, func() bool {
		var b []byte
		var err error
		if remote {
			var out string
			out, err = commandOutput(w.ctx, "ssh", "ubuntu", "cat "+quote(filepath.Join(home, "runtime.json"))+" 2>/dev/null")
			b = []byte(out)
		} else {
			b, err = os.ReadFile(filepath.Join(home, "runtime.json"))
		}
		return err == nil && json.Unmarshal(b, &cfg) == nil
	})
	return cfg, err
}
func (w *World) Await(timeout time.Duration, predicate func() bool) error {
	timer := time.NewTimer(timeout)
	defer timer.Stop()
	tick := time.NewTicker(25 * time.Millisecond)
	defer tick.Stop()
	for {
		if predicate() {
			return nil
		}
		select {
		case <-w.ctx.Done():
			return w.ctx.Err()
		case <-timer.C:
			return fmt.Errorf("world condition timed out")
		case <-tick.C:
		}
	}
}
func New(ctx context.Context, level string, replicas, seed int, blobOptions ...BlobOptions) (*World, error) {
	return newEnvironment(ctx, level, replicas, seed, false, "", NativeSetup{}, blobOptions...)
}
func NewReal(ctx context.Context, level string, replicas int, blobOptions ...BlobOptions) (*World, error) {
	return newEnvironment(ctx, level, replicas, 0, true, "", NativeSetup{}, blobOptions...)
}
func NewRealWithSetup(ctx context.Context, level string, replicas int, setup NativeSetup, blobOptions ...BlobOptions) (*World, error) {
	return newEnvironment(ctx, level, replicas, 0, true, "", setup, blobOptions...)
}
func newWorld(ctx context.Context, level string, replicas, seed int, gatewayBinary string, blobOptions ...BlobOptions) (*World, error) {
	return newEnvironment(ctx, level, replicas, seed, false, gatewayBinary, NativeSetup{}, blobOptions...)
}
func newEnvironment(ctx context.Context, level string, replicas, seed int, native bool, gatewayBinary string, setup NativeSetup, blobOptions ...BlobOptions) (*World, error) {
	root, err := os.Getwd()
	if err != nil {
		return nil, err
	}
	lab, err := os.Executable()
	if err != nil {
		return nil, err
	}
	id := strings.ToLower(contracts.NewID(""))
	id = id[len(id)-10:]
	w := &World{Dir: filepath.Join("/tmp/ablab", id), Level: level, Native: native, bridge: filepath.Join(root, "bin", "bridge"), lab: lab, ctx: ctx}
	if len(blobOptions) > 0 {
		w.blobOptions = blobOptions[0]
	}
	scope := "reminders:read,reminders:write,system:read"
	if len(blobOptions) > 0 {
		scope = "messages:read,reminders:read,system:read"
	}
	if err = os.MkdirAll(w.Dir, 0700); err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			w.Close()
		}
	}()
	if !native {
		w.previousSecretStore, w.previousSecretStoreSet = os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
		w.secretOverride = true
		os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	}
	options, _ := ctx.Value(optionsKey{}).(Options)
	peerHost := "127.0.0.1"
	w.NetworkPath = "loopback"
	if level == "replica" && options.DirectTailscale {
		address, err := commandOutput(ctx, "/Applications/Tailscale.app/Contents/MacOS/Tailscale", "ip", "-4")
		if err != nil {
			address, err = commandOutput(ctx, "tailscale", "ip", "-4")
		}
		if err != nil || net.ParseIP(address) == nil {
			return nil, fmt.Errorf("gateway Tailscale address unavailable")
		}
		peerHost = address
		w.NetworkPath = "tailscale_direct"
	}
	if level == "replica" {
		if _, err = commandOutput(ctx, "ssh", "ubuntu", "mkdir -p "+quote(w.Dir)+" && chmod 700 "+quote(w.Dir)); err != nil {
			return nil, err
		}
		w.remoteBinary = filepath.Join(w.Dir, "bridge")
		if _, err = commandOutput(ctx, "scp", filepath.Join(root, "bin", "bridge-linux-amd64"), "ubuntu:"+w.remoteBinary); err != nil {
			return nil, err
		}
		w.remoteLab = filepath.Join(w.Dir, "bridge-lab")
		if _, err = commandOutput(ctx, "scp", filepath.Join(root, "bin", "bridge-lab-linux-amd64"), "ubuntu:"+w.remoteLab); err != nil {
			return nil, err
		}
	}
	gatewayHome := filepath.Join(w.Dir, "gateway")
	if gatewayBinary == "" {
		gatewayBinary = w.bridge

	}
	if native {
		w.HelperApp = filepath.Join(w.Dir, "apps", "AppleBridgeHelper.app")
		w.TruthApp = filepath.Join(w.Dir, "apps", "AppleTruth.app")
		for _, build := range []struct{ script, app string }{{"build-helper.sh", w.HelperApp}, {"build-truth.sh", w.TruthApp}} {
			if _, err = commandOutput(ctx, filepath.Join(root, "packaging", "dev", build.script), build.app); err != nil {
				return nil, err
			}
		}
		w.nativeCleanup = setup.Cleanup
		if setup.BeforeStart != nil {
			if err = setup.BeforeStart(w); err != nil {
				return nil, err
			}
		}
		if setup.HoldBackfill {
			home := filepath.Join(gatewayHome, "helper")
			if err = os.MkdirAll(home, 0700); err != nil {
				return nil, err
			}
			if err = os.WriteFile(filepath.Join(home, "lab-hold-backfill"), []byte("hold"), 0600); err != nil {
				return nil, err
			}
		}
	}
	args := []string{"start", "--role", "gateway", "--home", gatewayHome, "--peer-addr", net.JoinHostPort(peerHost, "0"), "--api-addr", "127.0.0.1:0"}
	if gatewayBinary == w.bridge {
		args = append(args, "--blob-grace-ms", strconv.FormatInt(w.blobOptions.GraceMS, 10))
	}
	if native {
		args = append(args, "--helper-app", w.HelperApp, "--helper-lab-commands")
	}
	if _, err = w.start("gateway", false, gatewayHome, gatewayBinary, args...); err != nil {
		return nil, err
	}
	cfg, err := w.config(gatewayHome, false)
	if err != nil {
		return nil, err
	}
	w.proxy, err = NewFaultProxy(net.JoinHostPort(peerHost, "0"), cfg.PeerAddress)
	if err != nil {
		return nil, err
	}
	w.pairAddress = w.proxy.Address()
	if level == "replica" {
		direct := false
		if options.DirectTailscale {
			_, probeErr := commandOutput(ctx, "ssh", "ubuntu", "python3 -c "+quote("import socket; s=socket.create_connection("+fmt.Sprintf("(%q,%q)", peerHost, strings.Split(w.pairAddress, ":")[1])+",5); s.close()"))
			direct = probeErr == nil
		}
		if !direct {
			w.NetworkPath = "ssh_tunnel"
			if options.DirectTailscale {
				w.NetworkPath = "ssh_fallback_direct_tcp_timeout"
			}
			remotePort, portErr := commandOutput(ctx, "ssh", "ubuntu", "python3 -c "+quote("import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()"))
			if portErr != nil {
				return nil, portErr
			}
			if _, err = w.start("peer-tunnel", false, w.Dir, "ssh", "-N", "-o", "ExitOnForwardFailure=yes", "-R", "127.0.0.1:"+remotePort+":"+w.pairAddress, "ubuntu"); err != nil {
				return nil, err
			}
			w.pairAddress = net.JoinHostPort("127.0.0.1", remotePort)
			if err = w.Await(10*time.Second, func() bool {
				_, err := commandOutput(ctx, "ssh", "ubuntu", "python3 -c "+quote("import socket; s=socket.create_connection(('127.0.0.1',"+remotePort+"),2); s.close()"))
				return err == nil
			}); err != nil {
				return nil, err
			}
		}
	}

	token, err := w.execute(false, w.bridge, "token", "create", "--home", gatewayHome, "--scope", scope)
	if err != nil {
		return nil, err
	}
	w.Gateway = Endpoint{URL: "http://" + cfg.APIAddress, Host: cfg.APIAddress, Token: token, Home: gatewayHome}
	if !native {
		control := filepath.Join(w.Dir, "sim.sock")
		if _, err = w.start("simapple", false, w.Dir, w.lab, "simapple", "--socket", cfg.Socket, "--secret-file", filepath.Join(gatewayHome, "apple-secret"), "--control", control); err != nil {
			return nil, err
		}
		transport := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", control)
		}}
		w.Control = &http.Client{Transport: transport, Timeout: 5 * time.Second}
		if err = w.Await(10*time.Second, func() bool { _, err := w.Inspect(); return err == nil }); err != nil {
			return nil, err
		}
		if err = w.ControlAction("/seed", map[string]int{"count": seed}); err != nil {
			return nil, err
		}
		if err = w.Await(10*time.Second, func() bool { records, err := w.Records(w.Gateway); return err == nil && len(records) >= seed+2 }); err != nil {
			return nil, err
		}
	} else {
		if err = w.Await(15*time.Second, func() bool { _, err := os.Stat(filepath.Join(gatewayHome, "helper", "helper.pid")); return err == nil }); err != nil {
			return nil, err
		}
	}
	for i := 0; i < replicas; i++ {
		if err = w.AddReplicaScope(scope); err != nil {
			return nil, err
		}
	}

	if err = w.Await(15*time.Second, func() bool {
		for _, e := range w.Replicas {
			status, err := w.Status(e)
			if err != nil || status.State != "connected" {
				return false
			}
		}
		return true
	}); err != nil {
		return nil, err
	}
	success = true
	return w, nil
}
func freeAddress() (string, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return "", err
	}
	address := listener.Addr().String()
	listener.Close()
	return address, nil
}
func (w *World) Close() {
	if w.nativeCleanup != nil {
		cleanup := w.nativeCleanup
		w.nativeCleanup = nil
		if err := cleanup(w); err != nil {
			w.TruthFailures = append(w.TruthFailures, "native_cleanup_failed")
		}
	}
	if w.proxy != nil {
		w.proxy.Close()
	}
	for i := len(w.processes) - 1; i >= 0; i-- {
		p := w.processes[i]
		if p != nil {
			p.Stop()
			w.processes[i] = nil
		}
	}
	if w.Control != nil {
		w.Control.CloseIdleConnections()
	}
	if w.Level == "replica" {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		for _, endpoint := range w.Replicas {
			commandOutput(ctx, "ssh", "ubuntu", quote(w.remoteLab)+" credential-cleanup --home "+quote(endpoint.Home))
		}
		commandOutput(ctx, "ssh", "ubuntu", quote(w.remoteLab)+" credential-cleanup --home "+quote(filepath.Join(w.Dir, "pairing")))
		for _, home := range w.remoteExtraHomes {
			commandOutput(ctx, "ssh", "ubuntu", "rm -rf "+quote(home))
		}
		commandOutput(ctx, "ssh", "ubuntu", "rm -rf "+quote(w.Dir))
		cancel()
	}
	for _, home := range w.secretHomes {
		trust.NewSecretStore(home).Delete(context.Background(), "identity-key")
	}
	for _, endpoint := range append([]Endpoint{w.Gateway}, w.Replicas...) {
		if endpoint.Home != "" && !endpoint.Remote {
			trust.NewSecretStore(endpoint.Home).Delete(context.Background(), "identity-key")
		}
	}
	if w.diskMount != "" {
		commandOutput(context.Background(), "hdiutil", "detach", w.diskMount)
	}
	os.RemoveAll(w.Dir)
	if w.secretOverride {
		if w.previousSecretStoreSet {
			os.Setenv("APPLE_BRIDGE_SECRET_STORE", w.previousSecretStore)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SECRET_STORE")
		}
		w.secretOverride = false
	}
}
func (w *World) Partition(index int)  { w.proxy.Partition() }
func (w *World) Heal(index int) error { w.proxy.Heal(); return nil }
func (w *World) StopReplica(index int) {
	w.replicaProcesses[index].Stop()
	for j, p := range w.processes {
		if p == w.replicaProcesses[index] {
			w.processes[j] = nil
		}
	}
	w.replicaProcesses[index] = nil
}
func (w *World) RestartReplica(index int) error {
	e := w.Replicas[index]
	binary := w.bridge
	if e.Remote {
		binary = w.remoteBinary
	}
	_, err := w.execute(e.Remote, "rm", "-f", filepath.Join(e.Home, "runtime.json"))
	if err != nil {
		return err
	}
	_, port, _ := net.SplitHostPort(e.Host)
	p, err := w.start("replica"+strconv.Itoa(index), e.Remote, e.Home, binary, "start", "--role", "replica", "--home", e.Home, "--api-addr", "127.0.0.1:"+port, "--blob-cap", strconv.FormatInt(w.blobOptions.Cap, 10), "--blob-grace-ms", strconv.FormatInt(w.blobOptions.GraceMS, 10))
	if err != nil {
		return err
	}
	w.replicaProcesses[index] = p
	_, err = w.config(e.Home, e.Remote)
	return err
}
func (w *World) ControlAction(path string, action any) error {
	return w.controlAction(w.Control, path, action)
}
func (w *World) FlushCaptures(timeout time.Duration) error {
	client := *w.Control
	client.Timeout = timeout
	return w.controlAction(&client, "/flush", struct{}{})
}
func (w *World) controlAction(client *http.Client, path string, action any) error {
	b, err := json.Marshal(action)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(w.ctx, "POST", "http://sim"+path, bytes.NewReader(b))
	if err != nil {
		return err
	}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	if resp.StatusCode != 200 {
		return fmt.Errorf("sim control %s: status %d", path, resp.StatusCode)
	}
	return nil
}
func (w *World) Inspect() ([]contracts.Observation, error) {
	resp, err := w.Control.Get("http://sim/inspect")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var result []contracts.Observation
	err = json.NewDecoder(resp.Body).Decode(&result)
	return result, err
}

var HTTP = &http.Client{Timeout: 5 * time.Second}

func Request(ctx context.Context, e Endpoint, path string, input any, output any) (int, error) {
	method := "GET"
	var reader io.Reader
	if input != nil {
		method = "POST"
		b, err := json.Marshal(input)
		if err != nil {
			return 0, err
		}
		reader = bytes.NewReader(b)
	}
	req, err := http.NewRequestWithContext(ctx, method, e.URL+path, reader)
	if err != nil {
		return 0, err
	}
	req.Host = e.Host
	req.Header.Set("Authorization", "Bearer "+e.Token)
	req.Header.Set("Content-Type", "application/json")
	resp, err := HTTP.Do(req)
	if err != nil {
		return 0, err
	}
	defer resp.Body.Close()
	if output != nil {
		err = json.NewDecoder(resp.Body).Decode(output)
	} else {
		_, err = io.Copy(io.Discard, resp.Body)
	}
	if resp.StatusCode != 200 {
		return resp.StatusCode, fmt.Errorf("API status %d", resp.StatusCode)
	}
	return resp.StatusCode, err
}
func (w *World) Records(e Endpoint) ([]contracts.Record, error) {
	out := []contracts.Record{}
	after := ""
	for {
		var result contracts.ReadResult
		_, err := Request(w.ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 1000, After: after}, &result)
		if err != nil {
			return nil, err
		}
		out = append(out, result.Records...)
		if result.Next == "" {
			return out, nil
		}
		after = result.Next
	}
}

type ReplicaStatus struct {
	SnapshotPages    int    `json:"snapshot_pages"`
	Schema           int    `json:"schema"`
	SnapshotSequence uint64 `json:"snapshot_sequence"`
	State            string `json:"state"`
	Mode             string `json:"mode"`
	Snapshots        int    `json:"snapshots"`
	Resumes          int    `json:"resumes"`
}

func (w *World) Status(e Endpoint) (ReplicaStatus, error) {
	var result struct {
		Replica ReplicaStatus `json:"replica"`
	}
	_, err := Request(w.ctx, e, "/v1/op/sources.status", struct{}{}, &result)
	return result.Replica, err
}

func (w *World) ScopedToken(e Endpoint, scope string) (string, error) {
	binary := w.bridge
	if e.Remote {
		binary = w.remoteBinary
	}
	return w.execute(e.Remote, binary, "token", "create", "--home", e.Home, "--scope", scope)
}

func (w *World) Cursor(e Endpoint) (contracts.Cursor, error) {
	var result contracts.ReadResult
	_, err := Request(w.ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 1}, &result)
	return result.Cursor, err
}

func (w *World) BlobControl(action any, result any) error {
	transport := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "unix", filepath.Join(w.Gateway.Home, "run", "blobs-lab.sock"))
	}}
	defer transport.CloseIdleConnections()
	b, err := json.Marshal(action)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(w.ctx, "POST", "http://lab/", bytes.NewReader(b))
	if err != nil {
		return err
	}
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second}
	response, err := client.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return fmt.Errorf("blob lab status %d", response.StatusCode)
	}
	if result != nil {
		return json.NewDecoder(response.Body).Decode(result)
	}
	return nil
}
func (w *World) AddReplica() error { return w.AddReplicaScope("reminders:read,system:read") }
func (w *World) AddReplicaScope(scope string) error {
	i := len(w.Replicas)
	var err error
	remote := w.Level == "replica"
	home := filepath.Join(w.Dir, "replica"+strconv.Itoa(i))
	options, _ := w.ctx.Value(optionsKey{}).(Options)
	if w.Level == "replica" && options.PersistentReplicaStorage {
		var err error
		home, err = commandOutput(w.ctx, "ssh", "ubuntu", `mktemp -d "$HOME/.cache/apple-bridge-doctor.XXXXXX"`)
		if err != nil {
			return err
		}
		w.remoteExtraHomes = append(w.remoteExtraHomes, home)
	}
	binary := w.bridge
	if remote {
		binary = w.remoteBinary
		if _, err = commandOutput(w.ctx, "ssh", "ubuntu", "mkdir -p "+quote(home)); err != nil {
			return err
		}
	}
	code, err := w.execute(false, w.bridge, "pair", "--home", w.Gateway.Home, "--peer-addr", w.pairAddress, "--name", "replica"+strconv.Itoa(i), "--scope", scope)
	if err != nil {
		return err
	}
	if _, err = w.execute(remote, binary, "join", code, "--home", home); err != nil {
		return err
	}
	token, err := w.execute(remote, binary, "token", "create", "--home", home, "--scope", scope)
	if err != nil {
		return err
	}
	p, err := w.start("replica"+strconv.Itoa(i), remote, home, binary, "start", "--role", "replica", "--home", home, "--api-addr", "127.0.0.1:0", "--blob-cap", strconv.FormatInt(w.blobOptions.Cap, 10), "--blob-grace-ms", strconv.FormatInt(w.blobOptions.GraceMS, 10))
	if err != nil {
		return err
	}
	cfg, err := w.config(home, remote)
	if err != nil {
		return err
	}
	endpoint := Endpoint{URL: "http://" + cfg.APIAddress, Host: cfg.APIAddress, Home: home, Token: token, Remote: remote}
	if remote {
		addr, err := freeAddress()
		if err != nil {
			return err
		}
		_, port, _ := net.SplitHostPort(cfg.APIAddress)
		if _, err = w.start("tunnel"+strconv.Itoa(i), false, w.Dir, "ssh", "-N", "-o", "ExitOnForwardFailure=yes", "-L", addr+":127.0.0.1:"+port, "ubuntu"); err != nil {
			return err
		}
		endpoint.URL = "http://" + addr
	}
	w.Replicas = append(w.Replicas, endpoint)
	w.replicaProcesses = append(w.replicaProcesses, p)
	return nil
}

func (w *World) RecordsAt(e Endpoint) ([]contracts.Record, contracts.Cursor, error) {
	out := []contracts.Record{}
	after := ""
	var cursor contracts.Cursor
	for {
		var page contracts.ReadResult
		if _, err := Request(w.ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 1000, After: after}, &page); err != nil {
			return nil, cursor, err
		}
		if after == "" {
			cursor = page.Cursor
		} else if cursor != page.Cursor {
			return nil, cursor, fmt.Errorf("snapshot changed during pagination")
		}
		out = append(out, page.Records...)
		if page.Next == "" {
			return out, cursor, nil
		}
		after = page.Next
	}
}
