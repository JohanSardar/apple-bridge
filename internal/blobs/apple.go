package blobs

import (
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type appleSource struct {
	wire   *applepeer.Wire
	handle string
}

func (s appleSource) Read(ctx context.Context, d string, offset, length int64) ([]byte, bool, error) {
	return s.wire.ReadBlob(ctx, s.handle, offset, length)
}

type appleJob struct {
	variant bool
	ctx     context.Context
	source  appleSource
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
	if len(c.BlobHandles) == 0 {
		return
	}
	s.appleMu.Lock()
	defer s.appleMu.Unlock()
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.applePending == nil {
		s.applePending = map[string]*appleJob{}
	}
	variants := map[string]bool{}
	for _, observation := range c.Observations {
		for _, ref := range contracts.BlobReferences(observation.Data) {
			if ref.Variant {
				variants[ref.Digest] = true
			}
		}
	}
	for d, h := range c.BlobHandles {
		if !contracts.ValidDigest(d) {
			continue
		}
		e := s.manifest.Entries[d]
		if e.State == "present" || e.State == "failed" {
			continue
		}
		s.applePending[d] = &appleJob{ctx: ctx, source: appleSource{w, h}, variant: variants[d]}
	}
	if !s.appleRunning && len(s.applePending) > 0 {
		s.appleRunning = true
		go s.captureBlobs()
	}
}

func retryDelay(attempts int) time.Duration {
	return min(4*time.Second, 250*time.Millisecond<<min(max(0, attempts-1), 4))
}

func (s *Store) persistRetryLocked(d string) bool {
	if err := s.save(); err != nil {
		e := s.manifest.Entries[d]
		e.State, e.Reason = "failed", "retry_state_persist_failed"
		s.manifest.Entries[d] = e
		close(s.changed)
		s.changed = make(chan struct{})
		return false
	}
	return true
}

func (s *Store) captureBlobs() {
	for {
		s.appleMu.Lock()
		s.mu.Lock()
		var digest string
		var job *appleJob
		var ready time.Time
		for d, j := range s.applePending {
			if j.ctx.Err() != nil {
				continue
			}
			e := s.manifest.Entries[d]
			if job == nil || (j.variant && !job.variant && !e.NextAttemptAt.After(time.Now())) || (j.variant == job.variant && e.NextAttemptAt.Before(ready)) {
				digest, job, ready = d, j, e.NextAttemptAt
			}
		}
		s.mu.Unlock()
		if job == nil {
			s.appleRunning = false
			s.appleMu.Unlock()
			return
		}
		s.appleMu.Unlock()
		timer := time.NewTimer(max(0, time.Until(ready)))
		select {
		case <-job.ctx.Done():
			timer.Stop()
			continue
		case <-timer.C:
		}
		s.appleMu.Lock()
		if s.applePending[digest] != job || job.ctx.Err() != nil {
			s.appleMu.Unlock()
			continue
		}
		s.mu.Lock()
		e := s.manifest.Entries[digest]
		if e.State == "present" || e.State == "failed" {
			delete(s.applePending, digest)
			s.mu.Unlock()
			s.appleMu.Unlock()
			continue
		}
		if e.RetryAttempts >= 8 {
			e.State, e.Reason = "failed", "ipc_retry_exhausted"
			s.manifest.Entries[digest] = e
			close(s.changed)
			s.changed = make(chan struct{})
			s.persistRetryLocked(digest)
			delete(s.applePending, digest)
			s.mu.Unlock()
			s.appleMu.Unlock()
			continue
		}
		e.RetryAttempts++
		e.NextAttemptAt = time.Now().UTC().Add(retryDelay(e.RetryAttempts))
		s.manifest.Entries[digest] = e
		durable := s.persistRetryLocked(digest)
		if !durable {
			delete(s.applePending, digest)
		}
		s.mu.Unlock()
		s.appleMu.Unlock()
		if !durable {
			continue
		}
		err := s.Transfer(job.ctx, digest, job.source)
		s.appleMu.Lock()
		s.mu.Lock()
		e = s.manifest.Entries[digest]
		if err == nil || e.State != "pending" {
			e.NextAttemptAt = time.Time{}
			s.manifest.Entries[digest] = e
			s.persistRetryLocked(digest)
			delete(s.applePending, digest)
		} else if e.RetryAttempts >= 8 {
			e.State, e.Reason = "failed", "ipc_retry_exhausted"
			e.NextAttemptAt = time.Time{}
			s.manifest.Entries[digest] = e
			close(s.changed)
			s.changed = make(chan struct{})
			s.persistRetryLocked(digest)
			delete(s.applePending, digest)
		} else {
			e.NextAttemptAt = time.Now().UTC().Add(retryDelay(e.RetryAttempts))
			s.manifest.Entries[digest] = e
			if !s.persistRetryLocked(digest) {
				delete(s.applePending, digest)
			}
		}
		s.mu.Unlock()
		s.appleMu.Unlock()
	}
}
