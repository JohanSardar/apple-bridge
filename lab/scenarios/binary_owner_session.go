package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/url"
	"strings"
)

const localSessionCookie = "ab_session_local"

func binaryOwnerSession(ctx context.Context, w *world.World, kind string) (*webClient, error) {
	client := &webClient{home: w.Gateway.Home, ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1)}
	raw, err := w.GatewayCommand("web", "enroll", "--host", client.origin)
	if err != nil {
		return nil, err
	}
	fields := strings.Fields(string(raw))
	if len(fields) == 0 {
		return nil, fmt.Errorf("binary enrollment URL missing")
	}
	link, err := url.Parse(fields[0])
	if err != nil {
		return nil, err
	}
	if link.Fragment == "" {
		return nil, fmt.Errorf("binary enrollment token missing")
	}
	authenticator, err := newWebAuthenticator()
	if err != nil {
		return nil, err
	}
	status, _, err := client.signIn(authenticator, "enroll", link.Fragment, "trusted")
	if err != nil {
		return nil, err
	}
	if status != 200 {
		return nil, fmt.Errorf("binary enrollment status %d", status)
	}
	if kind == "public" {
		status, _, err = client.signIn(authenticator, "login", "", "public")
		if err != nil {
			return nil, err
		}
		if status != 200 {
			return nil, fmt.Errorf("binary public login status %d", status)
		}
	} else if kind != "trusted" {
		return nil, fmt.Errorf("invalid binary session kind")
	}
	if client.cookie == nil || client.cookie.Value == "" {
		return nil, fmt.Errorf("binary owner cookie missing")
	}
	return client, nil
}
