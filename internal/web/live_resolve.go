package web

import (
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type commandResolution struct {
	ID string
	At time.Time
}

func (s *server) allowResolve(session, id string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	now := s.rateTime()
	if s.resolutions == nil {
		s.resolutions = map[string][]commandResolution{}
	}
	if _, exists := s.resolutions[session]; !exists && len(s.resolutions) >= 8192 {
		for key, entries := range s.resolutions {
			if len(entries) == 0 || now.Sub(entries[len(entries)-1].At) >= time.Minute {
				delete(s.resolutions, key)
			}
		}
		if len(s.resolutions) >= 8192 {
			return false
		}
	}
	active := s.resolutions[session][:0]
	for _, entry := range s.resolutions[session] {
		if now.Sub(entry.At) < time.Minute {
			active = append(active, entry)
		}
	}
	s.resolutions[session] = active
	if len(active) >= 10 {
		return false
	}
	for _, entry := range active {
		if entry.ID == id && now.Sub(entry.At) < 5*time.Second {
			return false
		}
	}
	s.resolutions[session] = append(active, commandResolution{ID: id, At: now})
	return true
}

func (l *liveSocket) resolveCommand(id string) {
	fail := func(code, message string) {
		l.send(map[string]any{"t": "error", "id": id, "code": code, "message": message}, false)
	}
	if !contracts.ValidCommandID(id) {
		fail("invalid", "invalid command id")
		return
	}
	if l.server.deps.Operations.Target == nil {
		fail("unavailable", "commands unavailable")
		return
	}
	ctx, stop := context.WithTimeout(l.ctx, 15*time.Second)
	defer stop()
	status, err := l.localCommandStatus(ctx, id)
	if err != nil {
		problem := safeError(err)
		fail(problem.Code, problem.Message)
		return
	}
	throttled := !l.server.allowResolve(l.identity.Session.DeviceID, id)
	if !throttled {
		status, err = l.server.deps.Operations.Target.Status(ctx, l.identity.Principal, contracts.CommandQuery{ID: id, Resolve: true})
		if err != nil {
			problem := safeError(err)
			fail(problem.Code, problem.Message)
			return
		}
	}
	frame := map[string]any{"t": "cmd_status", "id": id, "status": status.State, "command": status, "error": status.Error}
	if throttled {
		frame["throttled"] = true
	}
	l.send(frame, false)
}
