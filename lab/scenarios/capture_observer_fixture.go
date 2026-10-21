package scenarios

import (
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func captureObserverChanges(w *world.World, source contracts.Source, collection, prefix string, count int) (contracts.Cursor, error) {
	observations := make([]contracts.Observation, 0, count)
	for n := 0; n < count; n++ {
		key := fmt.Sprintf("%s-%d", prefix, n)
		data := mustJSON(map[string]any{"title": "[ab-test capture observer] " + key, "text": "Bridge Test capture observer"})
		input := map[string]any{"key": key, "collection": collection, "data": data, "omit_capture": true}
		if err := w.ControlAction("/external-change", input); err != nil {
			return contracts.Cursor{}, err
		}
		observations = append(observations, contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, Data: data, ObservedAt: time.Now().UTC()})
	}
	before, err := w.Cursor(w.Gateway)
	if err != nil {
		return contracts.Cursor{}, err
	}
	if err := w.ControlAction("/capture-atomic", contracts.Capture{Source: source, Observations: observations}); err != nil {
		return contracts.Cursor{}, err
	}
	if err := w.FlushCaptures(5 * time.Second); err != nil {
		return contracts.Cursor{}, err
	}
	through, err := w.Cursor(w.Gateway)
	if err != nil {
		return through, err
	}
	width := captureSQL(w.Gateway, fmt.Sprintf("SELECT MAX(c.through_seq-COALESCE((SELECT MAX(p.through_seq) FROM journal_commits p WHERE p.epoch=c.epoch AND p.through_seq<c.through_seq),0)) FROM journal_commits c WHERE c.through_seq>%d AND c.through_seq<=%d", before.Sequence, through.Sequence))
	if through.Sequence-before.Sequence < uint64(count) || width < int64(count) {
		return through, contracts.ErrConflict
	}
	return through, nil
}
