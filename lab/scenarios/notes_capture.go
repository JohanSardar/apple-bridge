package scenarios

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func notesAwaitInitialCapture(w *world.World) error {
	return notesAwaitCapture(w.Await, func() ([]contracts.Record, error) { return w.Records(w.Gateway) })
}

func notesAwaitCapture(await func(time.Duration, func() bool) error, read func() ([]contracts.Record, error)) error {
	return await(30*time.Second, func() bool {
		rows, err := read()
		if err != nil {
			return false
		}
		for _, row := range rows {
			if row.Collection == "notes.folder" && !row.Deleted {
				return true
			}
		}
		return false
	})
}
