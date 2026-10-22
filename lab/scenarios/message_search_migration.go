package scenarios

import (
	"context"
	"database/sql"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"path/filepath"
	"reflect"
	"strings"
	"time"
)

func rmSearchMigration(ctx context.Context, w *world.World) (map[string]any, error) {
	metrics := map[string]any{"from": "3", "to": "4"}
	if w.Level != "sim" || w.Native {
		return metrics, fmt.Errorf("synthetic world required")
	}
	var before contracts.MessageSearchResult
	if err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "orch", Limit: 200}, &before); err != nil {
		return metrics, err
	}
	metrics["message_count"] = len(before.Items)
	w.StopGateway()
	database, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?_pragma=busy_timeout(5000)")
	if err != nil {
		return metrics, err
	}
	fields := []string{"text", "title", "notes", "plain_text", "given_name", "middle_name", "family_name", "nickname", "organization", "job_title"}
	expressions := make([]string, len(fields))
	for index, field := range fields {
		expressions[index] = fmt.Sprintf("coalesce(json_extract(r.payload,'$.data.%s'),'')", field)
	}
	_, err = database.ExecContext(ctx, "DELETE FROM records_fts; INSERT INTO records_fts(rowid,collection,id,source,text) SELECT m.fts_rowid,r.collection,r.id,r.source,"+strings.Join(expressions, "||' '||")+" FROM records r JOIN records_fts_map m ON m.collection=r.collection AND m.id=r.id WHERE r.deleted=0; UPDATE meta SET value='3' WHERE key='fts_version'")
	var oldMatches int
	if err == nil {
		err = database.QueryRowContext(ctx, "SELECT count(*) FROM records_fts WHERE records_fts MATCH ?", "Canop*").Scan(&oldMatches)
	}
	database.Close()
	metrics["legacy_filename_matches"] = oldMatches
	if err != nil {
		return metrics, err
	}
	if oldMatches != 0 {
		return metrics, fmt.Errorf("legacy projection unexpectedly contains filenames")
	}
	if err = w.RestartGateway(); err != nil {
		return metrics, err
	}
	var filenames contracts.MessageSearchResult
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "Canop", Limit: 200}, &filenames)
		if err == nil && len(filenames.Items) == 22 {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	metrics["rebuilt_filename_matches"] = len(filenames.Items)
	if err != nil {
		return metrics, err
	}
	if len(filenames.Items) != 22 {
		return metrics, fmt.Errorf("filename index did not rebuild")
	}
	var after contracts.MessageSearchResult
	if err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.search", contracts.MessageSearchQuery{Text: "orch", Limit: 200}, &after); err != nil {
		return metrics, err
	}
	ids := func(result contracts.MessageSearchResult) []string {
		out := make([]string, len(result.Items))
		for index, item := range result.Items {
			out[index] = item.ID
		}
		return out
	}
	metrics["message_ids_preserved"] = reflect.DeepEqual(ids(before), ids(after))
	if !reflect.DeepEqual(ids(before), ids(after)) {
		return metrics, fmt.Errorf("message IDs changed during migration")
	}
	database, err = sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db"))
	if err != nil {
		return metrics, err
	}
	defer database.Close()
	var version string
	err = database.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='fts_version'").Scan(&version)
	metrics["schema_version"] = version
	if err == nil && version != "4" {
		err = fmt.Errorf("current search schema unavailable")
	}
	return metrics, err
}
