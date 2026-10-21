package scenarios

import (
	"context"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webReplicaSession(ctx context.Context, w *world.World, cached **world.Endpoint) (map[string]any, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if *cached == nil {
		if err := w.AddReplicaScope("messages:write,system:read"); err != nil {
			return nil, err
		}
		endpoint := w.Replicas[len(w.Replicas)-1]
		*cached = &endpoint
	}
	endpoint := **cached
	target, err := w.Cursor(w.Gateway)
	if err != nil {
		return nil, err
	}
	err = w.Await(20*time.Second, func() bool {
		cursor, err := w.Cursor(endpoint)
		if err != nil || cursor.Epoch != target.Epoch || cursor.Sequence < target.Sequence {
			return false
		}
		status, err := w.Status(endpoint)
		return err == nil && status.State == "connected"
	})
	if err != nil {
		return nil, err
	}
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		return nil, err
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	origin := strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1)
	return map[string]any{"url": origin, "cookie": map[string]any{"name": bridgeweb.LocalSessionCookie, "value": token, "url": origin, "httpOnly": true, "secure": false, "sameSite": "Strict"}}, nil
}
