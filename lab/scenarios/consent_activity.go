package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"path/filepath"
)

func (c *webClient) activity(path string) (*http.Response, []byte, error) {
	store, err := state.Open(c.home)
	if err != nil {
		return nil, nil, err
	}
	defer store.Close()
	parsed, err := url.Parse(path)
	if err != nil {
		return nil, nil, err
	}
	rows, err := store.AgentActivity(c.ctx, parsed.Query().Get("principal"), parsed.Query().Get("operation"))
	if err != nil {
		return nil, nil, err
	}
	raw, err := json.Marshal(map[string]any{"activity": rows})
	return &http.Response{StatusCode: 200}, raw, err
}

func storedApproval(ctx context.Context, w *world.World, id string) contracts.CommandRecord {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "ledger.db")+"?mode=ro")
	if err != nil {
		return contracts.CommandRecord{}
	}
	defer db.Close()
	var raw []byte
	if db.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", id).Scan(&raw) != nil {
		return contracts.CommandRecord{}
	}
	var row contracts.CommandRecord
	json.Unmarshal(raw, &row)
	return row
}
