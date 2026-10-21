package web

import (
	"encoding/json"
	"net/http"
	"strconv"
)

func (s *server) sources(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("X-Bridge-Source-Sequence", strconv.FormatUint(s.sourceSeq.Add(1), 10))
	if s.deps.Operations == nil {
		reject(w, 503, "unavailable")
		return
	}
	result, err := s.deps.Operations.Invoke(r.Context(), current(r).Principal, "sources.status", json.RawMessage(`{}`))
	if err != nil {
		problem := safeError(err)
		status := map[string]int{"unauthorized": 401, "forbidden": 403, "unavailable": 503}[problem.Code]
		if status == 0 {
			status = 400
		}
		reject(w, status, problem.Code)
		return
	}
	respond(w, 200, result)
}
