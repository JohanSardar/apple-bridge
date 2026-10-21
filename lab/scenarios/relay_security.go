package scenarios

import (
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayProbeInput(w *world.World) map[string]any {
	replica := w.Replicas[0]
	return map[string]any{"URL": replica.URL, "Host": replica.Host, "Token": replica.Token, "Home": replica.Home, "ProxyReady": filepath.Join(w.Dir, "relay-route", "ready.json")}
}

func relaySecurityChecks(w *world.World, e *Evidence, add func(string, bool), enrollment *trust.Enrollment) {
	var err error
	var replay struct {
		Capture  bool `json:"capture_authenticated"`
		Nonce    bool `json:"fresh_nonce"`
		Rejected bool `json:"replay_rejected"`
	}
	err = w.RelayProbe("replay", relayProbeInput(w), &replay)
	add("captured_public_auth_frame_replay_rejected", err == nil && replay.Capture && replay.Nonce && replay.Rejected)
	if err != nil {
		return
	}
	var closed struct {
		At          int64   `json:"closed_gateway_wall_ns"`
		Uncertainty float64 `json:"clock_uncertainty_ms"`
	}
	var revokedAt int64
	err = w.RelayExchange("revocation", relayProbeInput(w), func() (any, error) {
		authority, err := trust.Open(w.Gateway.Home)
		if err != nil {
			return nil, err
		}
		revokedAt = time.Now().UnixNano()
		err = authority.Revoke(enrollment.ReplicaID)
		return map[string]bool{"revoked": err == nil}, err
	}, &closed)
	add("public_replica_feed_closes_on_revocation", err == nil && closed.At > 0)
	if err != nil {
		return
	}
	elapsed := float64(closed.At-revokedAt) / float64(time.Millisecond)
	e.Latency["revoke→public_feed_closed"] = stats([]float64{max(0, elapsed)}, 0)
	e.Metrics["revocation_clock_uncertainty_ms"] = closed.Uncertainty
	add("relay_revocation_closes_within_one_second", elapsed >= -closed.Uncertainty && elapsed+closed.Uncertainty < 1000)
	var local relayQuery
	err = w.Await(time.Second, func() bool {
		in := relayProbeInput(w)
		in["Collection"] = "reminders.reminder"
		return w.RelayProbe("query", in, &local) == nil && local.Status.State != "connected"
	})
	add("running_ubuntu_replica_reports_revoked_connection_closed", err == nil)
}
