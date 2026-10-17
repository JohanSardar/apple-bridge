package simapple

import (
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type durableApple struct {
	MessagesGeneration string                           `json:"messages_generation,omitempty"`
	Effects            map[string]int                   `json:"effects"`
	Evidence           map[string]contracts.IPCFrame    `json:"evidence"`
	Blobs              map[string][]byte                `json:"blobs,omitempty"`
	BlobState          string                           `json:"blob_state,omitempty"`
	BlobDelayNS        int64                            `json:"blob_delay_ns,omitempty"`
	InProgress         map[contracts.Source]bool        `json:"in_progress"`
	Generation         string                           `json:"generation"`
	Records            map[string]contracts.Observation `json:"records"`
	Revision           map[contracts.Source]uint64      `json:"revision"`
	Committed          map[contracts.Source]uint64      `json:"committed"`
	Checkpoints        map[contracts.Source]string      `json:"checkpoints"`
	Scans              int                              `json:"scans"`
}

func (f *fakeApple) saveLocked() error {
	f.durable.MessagesGeneration = f.messages.generation
	f.durable.Effects = f.effects
	f.durable.Evidence = f.evidence
	f.durable.Blobs = f.blobs
	f.durable.BlobState = f.blobState
	f.durable.BlobDelayNS = int64(f.blobDelay)
	data, err := json.Marshal(f.durable)
	if err != nil {
		return err
	}
	file, err := os.CreateTemp(filepath.Dir(f.path), "sim-state-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	if _, err = file.Write(data); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if err = os.Rename(file.Name(), f.path); err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(f.path))
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func (f *fakeApple) changedLocked(source contracts.Source) error {
	f.durable.Revision[source]++
	return f.saveLocked()
}

func (f *fakeApple) progressLocked() error {
	progress := durableApple{InProgress: f.durable.InProgress, Generation: f.durable.Generation, Committed: f.durable.Committed, Checkpoints: f.durable.Checkpoints, Scans: f.durable.Scans}
	data, err := json.Marshal(progress)
	if err != nil {
		return err
	}
	file, err := os.CreateTemp(filepath.Dir(f.path), "sim-progress-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	if _, err = file.Write(data); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	return os.Rename(file.Name(), filepath.Join(filepath.Dir(f.path), "sim-progress.json"))
}

func (f *fakeApple) committedLocked(source contracts.Source, revision uint64, full bool) {
	if f.completed == nil {
		f.completed = map[contracts.Source]map[uint64]bool{}
	}
	if f.completed[source] == nil {
		f.completed[source] = map[uint64]bool{}
	}
	if full {
		f.durable.Committed[source] = max(f.durable.Committed[source], revision)
	} else if revision > f.durable.Committed[source] {
		f.completed[source][revision] = true
	}
	for f.completed[source][f.durable.Committed[source]+1] {
		f.durable.Committed[source]++
	}
	for sequence := range f.completed[source] {
		if sequence <= f.durable.Committed[source] {
			delete(f.completed[source], sequence)
		}
	}
}
