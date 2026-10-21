package web

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"net/http"
	"strings"
)

func (s *server) agentRoutes(mux *http.ServeMux) {
	mux.HandleFunc("GET /web/v1/agents", func(w http.ResponseWriter, r *http.Request) {
		agents, err := s.deps.Authority.Agents()
		if err != nil {
			reject(w, 503, "unavailable")
			return
		}
		respond(w, 200, map[string]any{"agents": agents})
	})
	mux.HandleFunc("POST /web/v1/agents", func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			Name   string            `json:"name"`
			Level  trust.AgentLevel  `json:"level"`
			Scopes []contracts.Grant `json:"scopes"`
		}
		if json.NewDecoder(r.Body).Decode(&in) != nil {
			reject(w, 400, "invalid")
			return
		}
		agent, token, err := s.deps.Authority.CreateAgent(in.Name, in.Level, in.Scopes)
		if err != nil {
			reject(w, 400, "invalid")
			return
		}
		respond(w, 201, map[string]any{"agent": agent, "token": token, "url": servingOrigin(r)})
	})
	mux.HandleFunc("DELETE /web/v1/agents/{id}", func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		if !strings.HasPrefix(id, "agent:") || s.deps.Authority.Revoke(id) != nil {
			reject(w, 404, "not_found")
			return
		}
		respond(w, 200, map[string]bool{"ok": true})
	})
}
