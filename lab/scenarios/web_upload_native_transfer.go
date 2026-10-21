package scenarios

import (
	"bytes"
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

const messageTransferFinished = 5

func verifyNativeUploadTransfer(ctx context.Context, w *world.World, native *sql.DB, keys []string, expected []byte, metrics map[string]any, add func(string, bool)) {
	removed := uploadCommandStagingEmpty(w)
	add("handoff_removed_before_transfer_check", removed)
	if !removed {
		return
	}
	root, rootErr := filepath.EvalSymlinks(filepath.Join(os.Getenv("HOME"), "Library", "Messages", "Attachments"))
	attachments := 0
	for _, key := range keys {
		query := "SELECT COALESCE(a.transfer_state,-1),COALESCE(a.filename,'') FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?"
		var state int
		var path string
		if native.QueryRowContext(ctx, query, key).Scan(&state, &path) != nil {
			continue
		}
		attachments++
		metrics["attachment_transfer_state_at_handoff_cleanup"] = state
		inside, exact := false, false
		started := time.Now()
		err := w.Await(15*time.Second, func() bool {
			if native.QueryRowContext(ctx, query, key).Scan(&state, &path) != nil {
				return false
			}
			if strings.HasPrefix(path, "~/") {
				path = filepath.Join(os.Getenv("HOME"), path[2:])
			}
			resolved, problem := filepath.EvalSymlinks(path)
			relative, relativeErr := filepath.Rel(root, resolved)
			inside = rootErr == nil && problem == nil && relativeErr == nil && relative != "." && relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator))
			exact = false
			if inside {
				data, problem := os.ReadFile(resolved)
				exact = problem == nil && bytes.Equal(data, expected)
			}
			return state == messageTransferFinished && inside && exact
		})
		metrics["attachment_transfer_state_after_cleanup"] = state
		metrics["attachment_transfer_wait_ms"] = float64(time.Since(started)) / float64(time.Millisecond)
		add("native_transfer_finished_after_handoff_cleanup", err == nil && state == messageTransferFinished)
		add("native_attachment_is_in_messages_store", inside)
		add("native_attachment_bytes_exact_after_handoff_cleanup", exact)
	}
	add("exactly_one_native_attachment", attachments == 1)
}
