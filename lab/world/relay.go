package world

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (w *World) AddRelayReplica(scope, publicIP, probe string) error {
	root := filepath.Dir(w.bridge)
	w.remoteBinary = filepath.Join(w.Dir, "bridge-linux-amd64")
	home := filepath.Join(w.Dir, "relay-replica")
	routeHome := filepath.Join(w.Dir, "relay-route")
	if _, err := w.execute(true, "mkdir", "-p", home, routeHome); err != nil {
		return err
	}
	w.remoteExtraHomes = append(w.remoteExtraHomes, w.Dir)
	for _, copy := range []struct{ from, to string }{{filepath.Join(root, "bridge-linux-amd64"), w.remoteBinary}, {probe, filepath.Join(w.Dir, "relayprobe")}} {
		if _, err := commandOutput(w.ctx, "scp", copy.from, "ubuntu:"+copy.to); err != nil {
			return err
		}
	}
	encoded, err := w.execute(false, w.bridge, "pair", "--via-remote", "--scope", scope, "--home", w.Gateway.Home)
	if err != nil {
		return err
	}
	pair, err := trust.DecodePairCode(encoded)
	code := pair.Address
	if err != nil {
		return err
	}
	if !pair.Remote {
		return fmt.Errorf("remote pair code required")
	}
	ready := filepath.Join(routeHome, "ready.json")
	if _, err = w.start("relay-route", true, routeHome, filepath.Join(w.Dir, "relayprobe"), "proxy", "--host", code, "--ip", publicIP, "--ready", ready); err != nil {
		return err
	}
	var route struct {
		URL string `json:"url"`
	}
	if err = w.Await(10*time.Second, func() bool {
		raw, err := w.execute(true, "cat", ready)
		return err == nil && json.Unmarshal([]byte(raw), &route) == nil && route.URL != ""
	}); err != nil {
		return err
	}
	environment := []string{"HTTPS_PROXY=" + route.URL, "https_proxy=" + route.URL, "NO_PROXY=", "no_proxy="}
	join := append(append([]string{}, environment...), w.remoteBinary, "join", encoded, "--home", home)
	if _, err = w.execute(true, "env", join...); err != nil {
		return err
	}
	token, err := w.execute(true, w.remoteBinary, "token", "create", "--home", home, "--scope", scope)
	if err != nil {
		return err
	}
	args := append(append([]string{}, environment...), w.remoteBinary, "start", "--role", "replica", "--home", home, "--api-addr", "127.0.0.1:0", "--blob-cap", strconv.FormatInt(w.blobOptions.Cap, 10))
	process, err := w.start("relay-replica", true, home, "env", args...)
	if err != nil {
		return err
	}
	cfg, err := w.config(home, true)
	if err != nil {
		return err
	}
	w.Replicas = append(w.Replicas, Endpoint{URL: "http://" + cfg.APIAddress, Host: cfg.APIAddress, Home: home, Token: token, Remote: true})
	w.replicaProcesses = append(w.replicaProcesses, process)
	w.NetworkPath = "public Funnel via IP-pinned CONNECT; no peer or SSH tunnel"
	return nil
}
func (w *World) RelayProbe(action string, input, output any) error {
	raw, err := json.Marshal(input)
	if err != nil {
		return err
	}
	command := exec.CommandContext(w.ctx, "ssh", "ubuntu", "env APPLE_BRIDGE_SECRET_STORE=file "+quote(filepath.Join(w.Dir, "relayprobe"))+" "+quote(action))
	command.Stdin = bytes.NewReader(raw)
	response, err := command.Output()
	if err != nil {
		os.WriteFile(filepath.Join(w.Dir, "relay-probe.log"), response, 0600)
		return fmt.Errorf("relay probe failed: %w", err)
	}
	return json.Unmarshal(response, output)
}
