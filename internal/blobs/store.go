package blobs

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"sync"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Digest = string
type Pin = string
type Expect struct {
	Digest string
	Size   int64
}
type Source interface {
	Read(context.Context, string, int64, int64) ([]byte, bool, error)
}
type Config struct {
	Cap   int64
	Grace time.Duration
}
type Entry struct {
	ResumedBytes int64 `json:"resumed_bytes"`
	Corruptions  int   `json:"corruptions"`

	State          string    `json:"state"`
	Size           int64     `json:"size"`
	Received       int64     `json:"received"`
	UnreferencedAt time.Time `json:"unreferenced_at,omitempty"`
}
type BlobProgress struct {
	Entries      map[string]Entry `json:"entries"`
	Bytes        int64            `json:"bytes"`
	Backlog      int              `json:"backlog"`
	ResumedBytes int64            `json:"resumed_bytes"`
	Corruptions  int              `json:"corruptions"`
}
type manifest struct {
	Entries    map[string]Entry       `json:"entries"`
	Pins       map[string]string      `json:"pins"`
	References contracts.ReferenceSet `json:"references"`
}
type Store struct {
	mu       sync.Mutex
	transfer sync.Mutex
	root     string
	cfg      Config
	manifest manifest
	durable  []byte
}

func Open(home string, cfg Config) (*Store, error) {
	if cfg.Cap == 0 {
		cfg.Cap = 20 << 30
	}
	if cfg.Grace == 0 {
		cfg.Grace = 24 * time.Hour
	}
	s := &Store{root: filepath.Join(home, "blobs"), cfg: cfg, manifest: manifest{Entries: map[string]Entry{}, Pins: map[string]string{}, References: contracts.ReferenceSet{}}}
	for _, d := range []string{s.root, filepath.Join(s.root, "sha256"), filepath.Join(s.root, "staging")} {
		if err := os.MkdirAll(d, 0700); err != nil {
			return nil, err
		}
	}
	b, err := os.ReadFile(filepath.Join(s.root, "manifest.json"))
	if err == nil {
		s.durable = append([]byte(nil), b...)
		if err = json.Unmarshal(b, &s.manifest); err != nil {
			return nil, err
		}
	} else if !os.IsNotExist(err) {
		return nil, err
	}

	for d, e := range s.manifest.Entries {
		if !contracts.ValidDigest(d) {
			return nil, contracts.ErrCorrupt
		}
		if e.State == "present" {
			info, err := os.Stat(s.path(d))
			if os.IsNotExist(err) {
				e.State = "pending"
				e.Received = 0
			} else if err != nil {
				return nil, err
			} else if !info.Mode().IsRegular() {
				return nil, contracts.ErrCorrupt
			}
		}
		if e.State != "present" {
			if info, err := os.Stat(filepath.Join(s.root, "staging", d+".part")); err == nil {
				e.Received = info.Size()
			} else if !os.IsNotExist(err) {
				return nil, err
			} else {
				e.Received = 0
			}
		}
		s.manifest.Entries[d] = e
	}
	return s, nil
}
func (s *Store) path(d string) string { return filepath.Join(s.root, "sha256", d[:2], d[2:]) }
func (s *Store) save() error {
	b, err := json.Marshal(s.manifest)
	if err != nil {
		return err
	}
	if bytes.Equal(s.durable, b) {
		return nil
	}
	f, err := os.CreateTemp(s.root, "manifest-")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err = f.Write(b); err == nil {
		err = f.Sync()
	}
	if closeErr := f.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	if err = os.Rename(f.Name(), filepath.Join(s.root, "manifest.json")); err != nil {
		return err
	}
	if err = syncDir(s.root); err != nil {
		return err
	}
	s.durable = append(s.durable[:0], b...)
	return nil
}
func syncDir(path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return f.Sync()
}
func (s *Store) used() int64 {
	var n int64
	for _, e := range s.manifest.Entries {
		if e.State == "present" {
			n += e.Size
		} else {
			n += e.Received
		}
	}
	return n
}
func (s *Store) Stage(ctx context.Context, r io.Reader, expect Expect) (Digest, Pin, error) {
	s.transfer.Lock()
	defer s.transfer.Unlock()
	s.mu.Lock()
	defer s.mu.Unlock()
	if expect.Digest != "" && !contracts.ValidDigest(expect.Digest) {
		return "", "", contracts.ErrInvalid
	}
	f, err := os.CreateTemp(filepath.Join(s.root, "staging"), "stage-")
	if err != nil {
		return "", "", err
	}
	defer os.Remove(f.Name())
	defer f.Close()
	h := sha256.New()
	var total int64
	buf := make([]byte, 128<<10)
	for {
		if err = ctx.Err(); err != nil {
			return "", "", err
		}
		n, readErr := r.Read(buf)
		if n > 0 {
			total += int64(n)
			if s.cfg.Cap > 0 && s.used()+total > s.cfg.Cap {
				return "", "", contracts.ErrCapacity
			}
			if _, err = f.Write(buf[:n]); err != nil {
				return "", "", err
			}
			h.Write(buf[:n])
		}
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			return "", "", readErr
		}
	}
	d := hex.EncodeToString(h.Sum(nil))
	if expect.Digest != "" && d != expect.Digest || expect.Size > 0 && total != expect.Size {
		return "", "", contracts.ErrCorrupt
	}
	if err = f.Sync(); err != nil {
		return "", "", err
	}
	if err = f.Close(); err != nil {
		return "", "", err
	}
	if err = os.MkdirAll(filepath.Dir(s.path(d)), 0700); err != nil {
		return "", "", err
	}
	if err = os.Rename(f.Name(), s.path(d)); err != nil {
		return "", "", err
	}
	if err = syncDir(filepath.Dir(s.path(d))); err != nil {
		return "", "", err
	}
	pin := contracts.NewID("pin")
	s.manifest.Pins[pin] = d
	s.manifest.Entries[d] = Entry{State: "present", Size: total, Received: total}
	return d, pin, s.save()
}
func (s *Store) OpenAuthorized(ctx context.Context, grant trust.RecordGrant, digest Digest) (io.ReadSeekCloser, error) {
	if !contracts.ValidDigest(digest) || !grant.AllowsBlob(digest) {
		return nil, contracts.ErrUnauthorized
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.manifest.Entries[digest].State != "present" {
		return nil, contracts.ErrUnavailable
	}
	f, err := os.Open(s.path(digest))
	if os.IsNotExist(err) {
		return nil, contracts.ErrUnavailable
	}
	return f, err
}
func (s *Store) Protect(ctx context.Context, refs contracts.ReferenceSet) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.manifest.References = refs
	active := map[string]bool{}
	for _, list := range refs {
		for _, r := range list {
			if !contracts.ValidDigest(r.Digest) {
				return contracts.ErrInvalid
			}
			active[r.Digest] = true
			e, ok := s.manifest.Entries[r.Digest]
			if !ok {
				e = Entry{State: "pending", Size: r.Size}
				if r.State == "offloaded" || r.State == "missing" {
					e.State = r.State
				}
			}
			e.UnreferencedAt = time.Time{}
			s.manifest.Entries[r.Digest] = e
		}
	}
	for d, e := range s.manifest.Entries {
		if !active[d] && e.UnreferencedAt.IsZero() {
			e.UnreferencedAt = time.Now()
			s.manifest.Entries[d] = e
		}
	}
	if err := s.save(); err != nil {
		return err
	}
	for pin, d := range s.manifest.Pins {
		if active[d] {
			delete(s.manifest.Pins, pin)
		}
	}
	return s.save()
}
func (s *Store) Collect(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	changed := false
	pinned := map[string]bool{}
	for _, d := range s.manifest.Pins {
		pinned[d] = true
	}
	for d, e := range s.manifest.Entries {
		if err := ctx.Err(); err != nil {
			return err
		}
		if !pinned[d] && !e.UnreferencedAt.IsZero() && time.Since(e.UnreferencedAt) >= s.cfg.Grace {
			if err := os.Remove(s.path(d)); err != nil && !os.IsNotExist(err) {
				return err
			}
			os.Remove(filepath.Join(s.root, "staging", d+".part"))
			delete(s.manifest.Entries, d)
			changed = true
		}
	}
	if changed {
		return s.save()
	}
	return nil
}
func (s *Store) Progress() BlobProgress {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := BlobProgress{Entries: map[string]Entry{}, Bytes: s.used()}
	for d, e := range s.manifest.Entries {
		out.Entries[d] = e
		out.ResumedBytes += e.ResumedBytes
		out.Corruptions += e.Corruptions
		if e.State != "present" {
			out.Backlog++
		}
	}
	return out
}
func (s *Store) Transfer(ctx context.Context, d Digest, source Source) error {
	if !contracts.ValidDigest(d) {
		return contracts.ErrInvalid
	}
	s.transfer.Lock()
	defer s.transfer.Unlock()
	for attempt := 0; attempt < 3; attempt++ {
		err := s.transferOnce(ctx, d, source)
		if !errors.Is(err, contracts.ErrCorrupt) {
			return err
		}
		s.mu.Lock()
		e := s.manifest.Entries[d]
		e.State = "pending"
		e.Corruptions++
		e.Received = 0
		s.manifest.Entries[d] = e
		os.Remove(filepath.Join(s.root, "staging", d+".part"))
		s.save()
		s.mu.Unlock()
	}
	s.mu.Lock()
	e := s.manifest.Entries[d]
	e.State = "failed"
	s.manifest.Entries[d] = e
	s.save()
	s.mu.Unlock()
	return contracts.ErrCorrupt
}
func (s *Store) transferOnce(ctx context.Context, d Digest, source Source) (result error) {
	s.mu.Lock()
	e := s.manifest.Entries[d]
	if e.State == "present" {
		s.mu.Unlock()
		return nil
	}
	part := filepath.Join(s.root, "staging", d+".part")
	f, err := os.OpenFile(part, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		s.mu.Unlock()
		return err
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		s.mu.Unlock()
		return err
	}
	offset := info.Size()
	e.Received = offset
	if s.cfg.Cap > 0 && s.used()-s.manifest.Entries[d].Received+max(e.Size, offset) > s.cfg.Cap {
		e.State = "over_cap"
		s.manifest.Entries[d] = e
		s.save()
		s.mu.Unlock()
		return contracts.ErrCapacity
	}
	e.ResumedBytes += offset
	e.State = "pending"
	s.manifest.Entries[d] = e
	s.save()
	s.mu.Unlock()
	defer func() {
		if result == nil {
			return
		}
		s.mu.Lock()
		defer s.mu.Unlock()
		e := s.manifest.Entries[d]
		switch {
		case errors.Is(result, syscall.ENOSPC):
			e.State = "disk_full"
		case errors.Is(result, contracts.ErrCapacity):
			e.State = "over_cap"
		case errors.Is(result, contracts.ErrNotFound):
			e.State = "missing"
		case errors.Is(result, contracts.ErrUnsupported):
			e.State = "offloaded"
		case errors.Is(result, contracts.ErrCorrupt):
			e.State = "failed"
		}
		s.manifest.Entries[d] = e
		s.save()
	}()
	if _, err = f.Seek(offset, io.SeekStart); err != nil {
		return err
	}
	for {
		b, eof, err := source.Read(ctx, d, offset, 1<<20)
		if err != nil {
			return err
		}
		if len(b) > 1<<20 || len(b) == 0 && !eof {
			return contracts.ErrCorrupt
		}
		s.mu.Lock()
		over := s.cfg.Cap > 0 && s.used()+int64(len(b)) > s.cfg.Cap
		s.mu.Unlock()
		if over {
			return contracts.ErrCapacity
		}
		if _, err = f.Write(b); err != nil {
			return err
		}
		if err = f.Sync(); err != nil {
			return err
		}
		offset += int64(len(b))
		s.mu.Lock()
		e := s.manifest.Entries[d]
		e.Received = offset
		s.manifest.Entries[d] = e
		err = s.save()
		s.mu.Unlock()
		if err != nil {
			return err
		}
		if eof {
			break
		}
	}
	if _, err = f.Seek(0, io.SeekStart); err != nil {
		return err
	}
	h := sha256.New()
	if _, err = io.Copy(h, f); err != nil {
		return err
	}
	if hex.EncodeToString(h.Sum(nil)) != d {
		return contracts.ErrCorrupt
	}
	if err = f.Close(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err = os.MkdirAll(filepath.Dir(s.path(d)), 0700); err != nil {
		return err
	}
	if err = os.Rename(part, s.path(d)); err != nil {
		return err
	}
	if err = syncDir(filepath.Dir(s.path(d))); err != nil {
		return err
	}
	e = s.manifest.Entries[d]
	e.State = "present"
	e.Size = offset
	e.Received = offset
	s.manifest.Entries[d] = e
	return s.save()
}

func (s *Store) Release(ctx context.Context, pin Pin) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.manifest.Pins, pin)
	return s.save()
}
