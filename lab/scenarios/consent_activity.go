package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"path/filepath"
	"time"
)

func (c *webClient) activity(path string) (*http.Response, []byte, error) {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(c.home, "state.db")+"?mode=ro")
	if err != nil {
		return nil, nil, err
	}
	defer db.Close()
	parsed, err := url.Parse(path)
	if err != nil {
		return nil, nil, err
	}
	principal, operation := parsed.Query().Get("principal"), parsed.Query().Get("operation")
	result, err := db.QueryContext(c.ctx, `SELECT id,principal,operation,kind,state,at,count,argument_bytes,argument_hash FROM agent_activity WHERE (?='' OR principal=?) AND (?='' OR operation=?) AND at>=? ORDER BY at DESC,id DESC LIMIT 1000`, principal, principal, operation, operation, time.Now().Add(-90*24*time.Hour).Unix())
	if err != nil {
		return nil, nil, err
	}
	defer result.Close()
	rows := []map[string]any{}
	for result.Next() {
		var id, principal, operation, kind, state, hash string
		var at int64
		var count, argumentBytes int
		if err = result.Scan(&id, &principal, &operation, &kind, &state, &at, &count, &argumentBytes, &hash); err != nil {
			return nil, nil, err
		}
		rows = append(rows, map[string]any{"id": id, "principal": principal, "operation": operation, "kind": kind, "state": state, "at": time.Unix(at, 0).UTC(), "count": count, "argument_bytes": argumentBytes, "argument_hash": hash})
	}
	if err = result.Err(); err != nil {
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
