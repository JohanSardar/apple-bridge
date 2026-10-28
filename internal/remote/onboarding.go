package remote

import (
	"context"
	"fmt"
	"net"
	"time"

	"tailscale.com/ipn"
	"tailscale.com/ipn/ipnstate"
	"tailscale.com/tailcfg"
)

type LocalClient interface {
	StatusWithoutPeers(context.Context) (*ipnstate.Status, error)
	QueryFeature(context.Context, string) (*tailcfg.QueryFeatureResponse, error)
}

type EmbeddedNode interface {
	LocalClient() (LocalClient, error)
	ListenFunnel(string, string) (net.Listener, error)
	Close() error
}

type Options struct {
	Node    func(string, string) (EmbeddedNode, error)
	Probe   func(context.Context, string) (time.Time, error)
	Recheck time.Duration
}

func NewWithOptions(home string, options Options) *Remote {
	r := New(home)
	r.options = options
	return r
}

func (r *Remote) waitForFunnel(ctx context.Context, client LocalClient, status *Status) (*ipnstate.Status, error) {
	interval := r.options.Recheck
	if interval <= 0 {
		interval = 5 * time.Second
	}
	for {
		st, err := client.StatusWithoutPeers(ctx)
		if err != nil {
			return nil, fmt.Errorf("Tailscale approval check failed")
		}
		link := ""
		status.PendingLink = ""
		status.Message = ""
		if st.BackendState != "Running" {
			status.State = "login_pending"
			status.PendingLink = "login pending"
			link = st.AuthURL
			status.Message = "Waiting for Tailscale login"
		} else {
			status.State = "funnel_pending"
			if st.Self == nil {
				return nil, fmt.Errorf("Tailscale node has no identity")
			}
			response, err := client.QueryFeature(ctx, "funnel")
			if err != nil {
				return nil, fmt.Errorf("Tailscale approval check failed")
			}
			if response == nil {
				return nil, fmt.Errorf("empty Funnel feature response")
			}
			status.Message = "Waiting for Funnel approval"
			status.PendingLink = "Funnel approval pending"
			link = response.URL
			if response.Complete && ipn.CheckFunnelAccess(443, st.Self) == nil {
				return st, ctx.Err()
			}
		}
		r.publish(*status)
		r.approval(status, link)
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-time.After(interval):
		}
	}
}
