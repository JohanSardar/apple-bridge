package blobs

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"time"
)

type appleSource struct {
	wire   *applepeer.Wire
	handle string
}

func (s appleSource) Read(ctx context.Context, d string, offset, length int64) ([]byte, bool, error) {
	return s.wire.ReadBlob(ctx, s.handle, offset, length)
}

type appleJob struct {
	ctx      context.Context
	source   appleSource
	attempts int
	ready    time.Time
}

func (s *Store) ResumeCheckpoints(checkpoints map[contracts.Source]contracts.Checkpoint) map[contracts.Source]contracts.Checkpoint {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, e := range s.manifest.Entries {
		if e.State == "pending" {
			for source, checkpoint := range checkpoints {
				checkpoint.RefreshBlobs = true
				checkpoints[source] = checkpoint
			}
			return checkpoints
		}
	}
	return checkpoints
}
func (s *Store) Capture(ctx context.Context, c contracts.Capture, w *applepeer.Wire) {
	s.appleMu.Lock()
	defer s.appleMu.Unlock()
	if s.applePending == nil {
		s.applePending = map[string]*appleJob{}
	}
	for d, h := range c.BlobHandles {
		if contracts.ValidDigest(d) {
			e := s.Progress().Entries[d]
			if e.State == "present" || e.State == "failed" {
				continue
			}
			job := &appleJob{ctx: ctx, source: appleSource{w, h}}
			if previous := s.applePending[d]; previous != nil {
				job.attempts, job.ready = previous.attempts, previous.ready
			}
			s.applePending[d] = job
		}
	}
	if !s.appleRunning && len(s.applePending) > 0 {
		s.appleRunning = true
		go s.captureBlobs()
	}
}
func (s *Store) captureBlobs() {
	for {
		s.appleMu.Lock()
		var digest string
		var job *appleJob
		for d, j := range s.applePending {
			if j.ctx.Err() == nil {
				digest, job = d, j
				break
			}
		}
		if job == nil {
			s.appleRunning = false
			s.appleMu.Unlock()
			return
		}
		s.appleMu.Unlock()
		timer := time.NewTimer(max(0, time.Until(job.ready)))
		select {
		case <-job.ctx.Done():
			timer.Stop()
			continue
		case <-timer.C:
		}
		err := s.Transfer(job.ctx, digest, job.source)
		s.appleMu.Lock()
		current := s.applePending[digest]
		if current != job {
			s.appleMu.Unlock()
			continue
		}
		if err == nil {
			delete(s.applePending, digest)
		} else {
			job.attempts++
			state := s.Progress().Entries[digest].State
			if state != "pending" || job.attempts >= 8 {
				if state == "pending" {
					s.fail(digest, "ipc_retry_exhausted")
				}
				delete(s.applePending, digest)
			} else {
				job.ready = time.Now().Add(min(4*time.Second, 250*time.Millisecond<<min(job.attempts-1, 4)))
			}
		}
		s.appleMu.Unlock()
	}
}
func (s *Store) fail(d, reason string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	e := s.manifest.Entries[d]
	e.State, e.Reason = "failed", reason
	close(s.changed)
	s.changed = make(chan struct{})
	s.manifest.Entries[d] = e
	s.save()
}
