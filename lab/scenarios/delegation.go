package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func forgedDelegation(ctx context.Context, w *world.World, list string) bool {
	a, err := trust.Open(w.Replicas[0].Home)
	if err != nil {
		return false
	}
	p, err := a.AuthenticateToken(w.Replicas[0].Token)
	if err != nil {
		return false
	}
	p.Grants = append(p.Grants, "notes:write")
	args, _ := json.Marshal(map[string]any{"list_id": list, "title": "[ab-test] forged"})
	c := contracts.Command{ID: contracts.NewID("cmd"), Op: "reminders.create", Args: args}
	hash, _ := contracts.CommandHash(c)
	header, err := a.SignDelegation(p, c.ID, hash)
	if err != nil {
		return false
	}
	e, err := a.Enrollment()
	if err != nil {
		return false
	}
	b, _ := json.Marshal(map[string]any{"method": "submit", "command": c})
	req, err := http.NewRequestWithContext(ctx, "POST", "https://"+e.Address+"/v1/commands", bytes.NewReader(b))
	if err != nil {
		return false
	}
	req.Header.Set("X-Apple-Bridge-Delegation", header)
	transport := &http.Transport{TLSClientConfig: a.ClientTLS(e.Fingerprint)}
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
