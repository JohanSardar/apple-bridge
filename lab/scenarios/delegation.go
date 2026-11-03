package scenarios

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	labwire "github.com/JohanSardar/apple-bridge/lab/wire"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func forgedDelegation(ctx context.Context, w *world.World, list string) bool {
	replica := w.Replicas[0]
	if _, err := w.Records(replica); err != nil {
		return false
	}
	p, err := w.Principal(replica)
	if err != nil {
		return false
	}

	p.Grants = append(p.Grants, "notes:write")
	args, _ := json.Marshal(map[string]any{"list_id": list, "title": "[ab-test] forged"})
	c := contracts.Command{ID: contracts.NewID("cmd"), Op: "reminders.create", Args: args}
	hash, err := labwire.CommandHash(c.Op, c.Args, c.ExpectedVersion, c.ExpiresAt, c.AdmissionEra)
	if err != nil {
		return false
	}
	config, address, replicaID, err := w.FeedClient(replica)
	if err != nil || len(config.Certificates) == 0 {
		return false
	}
	key, ok := config.Certificates[0].PrivateKey.(ed25519.PrivateKey)
	if !ok {
		return false
	}
	grants := make([]string, len(p.Grants))
	for i, grant := range p.Grants {
		grants[i] = string(grant)
	}
	header, err := labwire.SignDelegation(key, labwire.Delegation{ReplicaID: replicaID, LocalPrincipal: p.ID, LocalGrants: grants, CommandID: c.ID, RequestHash: hash, IAT: time.Now().Unix()})
	if err != nil {
		return false
	}

	b, _ := json.Marshal(map[string]any{"method": "submit", "command": c})
	req, err := http.NewRequestWithContext(ctx, "POST", "https://"+address+"/v1/commands", bytes.NewReader(b))
	if err != nil {
		return false
	}
	req.Header.Set("X-Apple-Bridge-Delegation", header)
	transport := &http.Transport{TLSClientConfig: config}
	defer transport.CloseIdleConnections()
	resp, err := (&http.Client{Transport: transport}).Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	var result struct {
		Error *contracts.Error `json:"error"`
	}
	if json.NewDecoder(resp.Body).Decode(&result) != nil {
		return false
	}
	effects, err := w.Effects()
	return err == nil && result.Error != nil && result.Error.Code == "unauthorized" && effects[c.ID] == 0
}
