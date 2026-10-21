package scenarios

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadReplicaAbandon(w *world.World, c *webClient, add func(string, bool)) {
	status, uploaded := uploadRequest(c, "POST", "upload", bytes.NewBufferString("Bridge Test abandoned upload"), nil)
	gateway := uploadLeaseFixturePath(w.Gateway.Home, uploaded.ID)
	replica := uploadLeaseFixturePath(w.Replicas[0].Home, uploaded.ID)
	add("abandon_fixture_ready_at_both_endpoints", status == 200 && uploaded.State == "ready" && gateway != "" && replica != "")
	if gateway == "" || replica == "" {
		return
	}
	status, _ = uploadRequest(c, "DELETE", "upload/"+uploaded.ID, nil, nil)
	add("replica_abandon_forwarded_to_gateway", status == 200)
	_, gatewayErr := os.Stat(gateway)
	_, replicaErr := os.Stat(replica)
	add("abandon_removes_both_owned_leases", os.IsNotExist(gatewayErr) && os.IsNotExist(replicaErr))
	status, _ = uploadRequest(c, "GET", "upload/"+uploaded.ID, nil, nil)
	add("abandoned_replica_upload_unavailable", status == 404)
}
func uploadLeaseFixturePath(home, id string) string {
	paths, _ := filepath.Glob(filepath.Join(home, "blobs/upload-leases/*.json"))
	for _, path := range paths {
		raw, _ := os.ReadFile(path)
		var lease struct {
			ID string `json:"upload_id"`
		}
		if json.Unmarshal(raw, &lease) == nil && lease.ID == id {
			return path
		}
	}
	return ""
}
