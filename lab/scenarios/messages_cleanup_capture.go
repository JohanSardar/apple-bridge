package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"sync"
	"time"
)

func captureMessageCreations(ctx context.Context, db *sql.DB, baseline int64, run, path string) func() []string {
	var mu sync.Mutex
	ids := map[string]bool{}
	tag := "[ab-test " + run + "]"
	go func() {
		ticker := time.NewTicker(20 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				rows, err := db.QueryContext(ctx, `SELECT DISTINCT m.guid FROM message m LEFT JOIN message_attachment_join j ON j.message_id=m.ROWID LEFT JOIN attachment a ON a.ROWID=j.attachment_id WHERE m.ROWID>? AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0 OR instr(a.filename,?)>0)`, baseline, tag, tag, "ab-"+run+".")
				if err != nil {
					continue
				}
				for rows.Next() {
					var id string
					if rows.Scan(&id) == nil {
						mu.Lock()
						ids[id] = true
						mu.Unlock()
					}
				}
				rows.Close()
				mu.Lock()
				captured := make([]string, 0, len(ids))
				for id := range ids {
					captured = append(captured, id)
				}
				mu.Unlock()
				data, problem := json.Marshal(map[string]any{"run_id": run, "ids": captured})
				if problem == nil && os.WriteFile(path+".tmp", data, 0600) == nil {
					_ = os.Rename(path+".tmp", path)
				}
			}
		}
	}()
	return func() []string {
		mu.Lock()
		defer mu.Unlock()
		result := make([]string, 0, len(ids))
		for id := range ids {
			result = append(result, id)
		}
		return result
	}
}
