package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadSendDiagnostic(ctx context.Context, w *world.World, e *Evidence, direction string, command contracts.Command, err error) {
	diagnostic := map[string]any{"error": err.Error(), "command_id": command.ID}
	persisted, readErr := uploadGatewayCommand(ctx, w.Gateway.Home, command.ID)
	diagnostic["gateway_state"] = persisted.State
	diagnostic["gateway_record_error_type"] = fmt.Sprintf("%T", readErr)
	diagnostic["gateway_cursor"] = persisted.Cursor
	follow, _ := w.Status(w.Replicas[0])
	diagnostic["replica_status"] = follow
	cursor, cursorErr := w.Cursor(w.Replicas[0])
	diagnostic["replica_cursor"] = cursor
	diagnostic["replica_cursor_error_type"] = fmt.Sprintf("%T", cursorErr)
	raw, _ := json.Marshal(diagnostic)
	path := filepath.Join("lab/evidence/private", e.RunID+"-"+direction+"-send.json")
	if os.MkdirAll(filepath.Dir(path), 0700) == nil {
		e.Metrics[direction+"_private_diagnostic_written"] = os.WriteFile(path, raw, 0600) == nil
	}
}

func uploadGatewayCommand(ctx context.Context, home, id string) (contracts.CommandRecord, error) {
	var record contracts.CommandRecord
	database, err := sql.Open("sqlite", "file:"+filepath.Join(home, "ledger.db")+"?mode=ro")
	if err != nil {
		return record, err
	}
	defer database.Close()
	var raw []byte
	if err = database.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", id).Scan(&raw); err != nil {
		return record, err
	}
	err = json.Unmarshal(raw, &record)
	return record, err
}
