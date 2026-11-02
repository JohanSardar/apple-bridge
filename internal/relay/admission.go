package relay

import (
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"net"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"
)

func (s *Store) RequireOwnerKeys(path string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.admissionFile = path
	if path == "" {
		s.admissionFile = "/dev/null"
	}
}
func (s *Store) ownerKeys() (map[string]bool, error) {
	data, err := os.ReadFile(s.admissionFile)
	if err != nil || len(data) > 16384 {
		return nil, errors.New("owner key configuration unavailable")
	}
	keys := map[string]bool{}
	for _, field := range strings.Fields(string(data)) {
		key, err := base64.RawURLEncoding.DecodeString(field)
		if err != nil || len(key) != ed25519.PublicKeySize || base64.RawURLEncoding.EncodeToString(key) != field {
			return nil, errors.New("invalid owner key configuration")
		}
		keys[field] = true
	}
	if len(keys) == 0 {
		return nil, errors.New("empty owner key configuration")
	}
	return keys, nil
}
func (s *Store) ValidateOwnerKeys() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	_, err := s.ownerKeys()
	return err
}
func (s *Store) admitted(key string) bool {
	if s.admissionFile == "" {
		return true
	}
	keys, err := s.ownerKeys()
	return err == nil && keys[key]
}

type sourceBudget struct {
	until       time.Time
	challenges  int
	enrollments int
}

func admissionThrottle(next http.Handler) http.Handler {
	var mu sync.Mutex
	sources := map[string]sourceBudget{}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		challenge := r.URL.Path == "/challenge"
		enroll := r.URL.Path == "/enroll" || r.URL.Path == "/rotate"
		if !challenge && !enroll {
			next.ServeHTTP(w, r)
			return
		}
		source, _, err := net.SplitHostPort(r.RemoteAddr)
		if err != nil {
			http.Error(w, "invalid source", 400)
			return
		}
		now := time.Now()
		mu.Lock()
		for k, v := range sources {
			if !now.Before(v.until) {
				delete(sources, k)
			}
		}
		b, exists := sources[source]
		allowed := exists || len(sources) < 4096
		if !exists {
			b.until = now.Add(time.Minute)
		}
		if challenge {
			allowed = allowed && b.challenges < 30
			if allowed {
				b.challenges++
			}
		} else {
			allowed = allowed && b.enrollments < 20
			if allowed {
				b.enrollments++
			}
		}
		if allowed {
			sources[source] = b
		}
		mu.Unlock()
		if !allowed {
			w.Header().Set("Retry-After", "60")
			http.Error(w, "busy", 429)
			return
		}
		next.ServeHTTP(w, r)
	})
}
