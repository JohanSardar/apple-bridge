package demo

import (
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func Populate(w *world.World, dataset Dataset) error {
	previous := w.Control.Timeout
	w.Control.Timeout = 2 * time.Minute
	defer func() { w.Control.Timeout = previous }()
	return w.ControlAction("/dataset", dataset.Dataset)
}

func PopulateChunked(w *world.World, dataset Dataset, limit int) error {
	if limit < 64 || limit%64 != 0 {
		return fmt.Errorf("dataset chunk must contain whole 64-observation capture units")
	}
	previous := w.Control.Timeout
	w.Control.Timeout = 2 * time.Minute
	defer func() { w.Control.Timeout = previous }()
	first := true
	for _, source := range []contracts.Source{contracts.Contacts, contracts.Messages, contracts.Reminders, contracts.Calendar} {
		records := make([]contracts.Observation, 0)
		positions := map[string]int{}
		for _, record := range dataset.Records {
			if contracts.CollectionSource(record.Collection) == source {
				positions[record.NativeKey] = len(records)
				records = append(records, record)
			}
		}
		for start := 0; start < len(records); {
			end := min(start+limit, len(records))
			for {
				extended := end
				for still, movie := range dataset.LivePhotos {
					a, aok := positions[still]
					b, bok := positions[movie]
					if aok && bok && (a >= start && a < end || b >= start && b < end) {
						extended = max(extended, min(((max(a, b)/64)+1)*64, len(records)))
					}
				}
				if extended == end {
					break
				}
				end = extended
			}
			part := simapple.Dataset{Records: records[start:end], LivePhotos: map[string]string{}}
			if first {
				part.Blobs = dataset.Blobs
				first = false
			}
			for still, movie := range dataset.LivePhotos {
				a, aok := positions[still]
				b, bok := positions[movie]
				if aok && bok && a >= start && a < end && b >= start && b < end {
					part.LivePhotos[still] = movie
				}
			}
			if len(part.LivePhotos) > 0 {
				part.Blobs = dataset.Blobs
			}
			if err := w.ControlAction("/dataset", part); err != nil {
				return fmt.Errorf("dataset %s observations %d..%d: %w", source, start, end, err)
			}
			start = end
		}
	}
	return nil
}
