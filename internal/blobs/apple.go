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
	live    bool
	order   uint64
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
	variants := map[string]bool{}
	for _, observation := range c.Observations {
		for _, ref := range contracts.BlobReferences(observation.Data) {
			if ref.Variant {
				variants[ref.Digest] = true
			}
		}
	}
	s.appleMu.Lock()
	defer s.appleMu.Unlock()
	if s.applePending == nil {
		s.applePending = map[string]*appleJob{}
	}
	for d, h := range c.BlobHandles {
		if !contracts.ValidDigest(d) {
			continue
		}
		s.appleSequence++
		job := &appleJob{ctx: ctx, source: appleSource{w, h}, variant: variants[d], live: c.Live, order: s.appleSequence}
		if previous := s.applePending[d]; previous != nil && previous.ctx.Err() == nil {
			job.order = previous.order
			job.live = job.live || previous.live
			job.variant = job.variant || previous.variant
		}
		s.applePending[d] = job
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

func (s *Store) discardAppleJob(digest string, job *appleJob) {
	s.appleMu.Lock()
	defer s.appleMu.Unlock()
	if s.applePending[digest] == job {
		delete(s.applePending, digest)
	}
}

func (s *Store) captureBlobs() {
	for {
		s.appleMu.Lock()
		jobs := make(map[string]*appleJob, len(s.applePending))
		for d, job := range s.applePending {
			if job.ctx.Err() != nil {
				delete(s.applePending, d)
			} else {
				jobs[d] = job
			}
		}
		if len(jobs) == 0 {
			s.appleRunning = false
			s.appleMu.Unlock()
			return
		}
		s.appleMu.Unlock()
		s.mu.Lock()
		var digest string
		var job *appleJob
		var ready time.Time
		now := time.Now()
		for d, j := range jobs {
			e := s.manifest.Entries[d]
			available := !e.NextAttemptAt.After(now)
			readyFirst := available && ready.After(now)
			prioritized := job != nil && available && !ready.After(now) && (j.variant && !job.variant || j.variant == job.variant && (j.live && !job.live || j.live == job.live && j.order < job.order))
			earlier := !available && ready.After(now) && e.NextAttemptAt.Before(ready)
			if job == nil || readyFirst || prioritized || earlier {
				digest, job, ready = d, j, e.NextAttemptAt
			}
		}
		s.mu.Unlock()
		timer := time.NewTimer(max(0, time.Until(ready)))
		select {
		case <-job.ctx.Done():
			timer.Stop()
			continue
		case <-timer.C:
		}
		s.appleMu.Lock()
		current := s.applePending[digest] == job && job.ctx.Err() == nil
		s.appleMu.Unlock()
		if !current {
			continue
		}
		s.mu.Lock()
		e := s.manifest.Entries[digest]
		if e.State == "present" || e.State == "failed" {
			s.mu.Unlock()
			s.discardAppleJob(digest, job)
			continue
		}
		if e.RetryAttempts >= 8 {
			e.State, e.Reason = "failed", "ipc_retry_exhausted"
			s.manifest.Entries[digest] = e
			close(s.changed)
			s.changed = make(chan struct{})
			s.persistRetryLocked(digest)
			s.mu.Unlock()
			s.discardAppleJob(digest, job)
			continue
		}
		e.RetryAttempts++
		e.NextAttemptAt = time.Now().UTC().Add(retryDelay(e.RetryAttempts))
		s.manifest.Entries[digest] = e
		durable := s.persistRetryLocked(digest)
		s.mu.Unlock()
		if !durable {
			s.discardAppleJob(digest, job)
			continue
		}
		err := s.Transfer(job.ctx, digest, job.source)
		s.mu.Lock()
		e = s.manifest.Entries[digest]
		remove := false
		if err == nil || e.State != "pending" {
			e.NextAttemptAt = time.Time{}
			s.manifest.Entries[digest] = e
			s.persistRetryLocked(digest)
			remove = true
		} else if e.RetryAttempts >= 8 {
			e.State, e.Reason = "failed", "ipc_retry_exhausted"
			e.NextAttemptAt = time.Time{}
			s.manifest.Entries[digest] = e
			close(s.changed)
			s.changed = make(chan struct{})
			s.persistRetryLocked(digest)
			remove = true
		} else {
			e.NextAttemptAt = time.Now().UTC().Add(retryDelay(e.RetryAttempts))
			s.manifest.Entries[digest] = e
			remove = !s.persistRetryLocked(digest)
		}
		s.mu.Unlock()
		if remove {
			s.discardAppleJob(digest, job)
		}
	}
}
