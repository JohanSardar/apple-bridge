package web

import (
	"encoding/json"
	"net/http"
)

func (s *server) sources(w http.ResponseWriter, r *http.Request) {
	if s.deps.Operations == nil {
		reject(w, 503, "unavailable")
		return
	}
	result, err := s.deps.Operations.Invoke(r.Context(), current(r).Principal, "sources.status", json.RawMessage(`{}`))
	if err != nil {
		reject(w, 400, safeError(err).Code)
		return
	}
	respond(w, 200, result)
}
