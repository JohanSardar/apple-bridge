package web

import (
	"encoding/json"
	"io"
	"net/http"
)

func (s *server) handles(w http.ResponseWriter, r *http.Request) {
	var args struct {
		IDs []string `json:"ids"`
	}
	decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 64<<10))
	if decoder.Decode(&args) != nil || decoder.Decode(new(any)) != io.EOF || len(args.IDs) > 500 {
		reject(w, 400, "invalid")
		return
	}
	if s.deps.Operations == nil {
		reject(w, 503, "unavailable")
		return
	}
	input, _ := json.Marshal(args)
	result, err := s.deps.Operations.Invoke(r.Context(), current(r).Principal, "messages.handles", input)
	if err != nil {
		reject(w, 400, safeError(err).Code)
		return
	}
	respond(w, 200, result)
}
