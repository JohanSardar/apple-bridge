package scenarios

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func runRemotePrivateReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: "private-live", NetworkPath: "system tailnet TCP transport to live private embedded node"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0 && e.Status != "limited"
		for _, a := range e.Assertions {
			if !a.Passed {
				e.Passed = false
			}
		}
	}()
	home, err := os.UserHomeDir()
	add("live_home_available", err == nil)
	if err != nil {
		return
	}
	home = filepath.Join(home, "Library", "Application Support", "AppleBridgeLive")
	command := func(parent context.Context, action string) error {
		bounded, cancel := context.WithTimeout(parent, 30*time.Second)
		defer cancel()
		return remote.CommandWithOpener(bounded, home, action, "private", "", io.Discard, func(context.Context, string) error { return fmt.Errorf("interactive login unavailable") })
	}
	status := func() (remote.Status, error) {
		var out bytes.Buffer
		err := remote.Command(ctx, home, "status", "", "", &out)
		var st remote.Status
		if err == nil {
			err = json.Unmarshal(out.Bytes(), &st)
		}
		return st, err
	}
	defer func() {
		restoreCtx, cancel := context.WithTimeout(context.Background(), 35*time.Second)
		defer cancel()
		restored := command(restoreCtx, "on") == nil
		st, err := status()
		add("private_on_restored", restored && err == nil && st.State == "on" && st.Backend == "private")
	}()
	err = command(ctx, "off")
	add("fresh_invitation_off", err == nil)
	if err != nil {
		return
	}
	err = command(ctx, "on")
	add("private_on", err == nil)
	if err != nil {
		return
	}
	st, err := status()
	add("status_on_private", err == nil && st.State == "on" && st.Backend == "private")
	if err != nil || st.State != "on" || st.Backend != "private" {
		return
	}
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimPrefix(st.Message, "bridge connect "))
	var invite struct {
		Host   string
		Secret string
	}
	valid := err == nil && json.Unmarshal(raw, &invite) == nil && strings.HasSuffix(invite.Host, ".ts.net") && !strings.ContainsAny(invite.Host, "/:@?# ") && len(invite.Secret) == 43
	add("private_invitation_valid", valid)
	if !valid {
		return
	}
	address := net.JoinHostPort(invite.Host, "7780")
	dial := func() (net.Conn, error) {
		return (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, "tcp", address)
	}
	conn, dialErr := dial()
	if conn != nil {
		conn.Close()
	}
	timeout := false
	if dialErr != nil {
		if ne, ok := dialErr.(net.Error); ok {
			timeout = ne.Timeout() && ctx.Err() == nil
		}
		if !timeout {
			add("tailnet_tcp_reachable", false)
			return
		}
	}
	if s.ID == "RMT-STATUS-OFF" {
		err = command(ctx, "off")
		add("remote_off", err == nil)
		st, statusErr := status()
		add("status_off", statusErr == nil && st.State == "off")
		conn, stoppedErr := dial()
		if conn != nil {
			conn.Close()
		}
		add("port_stops_answering", stoppedErr != nil && ctx.Err() == nil)
	} else if !timeout {
		transport := &http.Transport{DialContext: (&net.Dialer{Timeout: 5 * time.Second}).DialContext, DisableKeepAlives: true}
		defer transport.CloseIdleConnections()
		client := &http.Client{Transport: transport, Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
		request := func(method, path, secret, session string) (*http.Response, error) {
			req, err := http.NewRequestWithContext(ctx, method, "http://"+address+path, nil)
			if err != nil {
				return nil, err
			}
			req.Host = "localhost:7780"
			if secret != "" {
				req.Header.Set("Authorization", "Bearer "+secret)
			}
			if session != "" {
				req.Header.Set("X-Bridge-Private-Session", session)
			}
			return client.Do(req)
		}
		response, err := request("GET", "/", "", "")
		rejected := err == nil && response.StatusCode == 403
		if response != nil {
			response.Body.Close()
		}
		add("session_required", rejected)
		if !rejected {
			return
		}
		response, err = request("POST", "/_bridge/private/invite", invite.Secret, "")
		var grant struct{ Session string }
		redeemed := false
		if response != nil {
			redeemed = err == nil && response.StatusCode == 200 && json.NewDecoder(io.LimitReader(response.Body, 4096)).Decode(&grant) == nil && grant.Session != ""
			response.Body.Close()
		}
		add("invitation_redeemed", redeemed)
		if !redeemed {
			return
		}
		response, err = request("GET", "/mcp", "", grant.Session)
		reached := err == nil && response.StatusCode == 401 && strings.Contains(response.Header.Get("WWW-Authenticate"), "Bearer")
		if response != nil {
			response.Body.Close()
		}
		add("session_reaches_bridge_challenge", reached)
	}
	if timeout {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "tailnet_tcp_grant", Status: "limited", Detail: "limited: needs tailnet grant tcp:7780 to bridge node"})
	}
	return
}
