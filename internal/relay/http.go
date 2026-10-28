package relay

import (
	"encoding/json"
	"net/http"
	"strings"
)

func (s *Server) Handler(admin string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /tunnel", s.tunnel)
	mux.HandleFunc("GET /challenge", func(w http.ResponseWriter, r *http.Request) {
		nonce, err := s.Store.Challenge()
		if err != nil {
			http.Error(w, "busy", 429)
			return
		}
		reply(w, map[string]string{"nonce": nonce})
	})
	mux.HandleFunc("POST /enroll", func(w http.ResponseWriter, r *http.Request) {
		var in Enrollment
		if !decode(w, r, &in) {
			return
		}
		c, err := s.Store.Enroll(in)
		if err != nil {
			http.Error(w, "enrollment refused", 403)
			return
		}
		s.Disconnect(c.Hostname)
		reply(w, c)
	})
	mux.HandleFunc("POST /rotate", func(w http.ResponseWriter, r *http.Request) {
		var in Enrollment
		if !decode(w, r, &in) {
			return
		}
		c, err := s.Store.Enroll(in)
		if err != nil {
			http.Error(w, "rotation refused", 403)
			return
		}
		s.Disconnect(c.Hostname)
		reply(w, c)
	})
	mux.HandleFunc("POST /revoke", func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			Hostname string `json:"hostname"`
		}
		if !decode(w, r, &in) {
			return
		}
		token := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		if !s.Store.authenticated(in.Hostname, token) {
			http.Error(w, "denied", 403)
			return
		}
		d, _ := s.Store.Device(in.Hostname)
		if s.SetPolicy(in.Hostname, d.Limits, d.Disabled, true) != nil {
			http.Error(w, "storage unavailable", 503)
			return
		}
		w.WriteHeader(204)
	})
	mux.HandleFunc("POST /policy", func(w http.ResponseWriter, r *http.Request) {
		if admin == "" || digest(strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")) != digest(admin) {
			http.Error(w, "denied", 403)
			return
		}
		var in struct {
			Hostname string `json:"hostname"`
			Limits   Limits `json:"limits"`
			Disabled bool   `json:"disabled"`
			Revoked  bool   `json:"revoked"`
		}
		if !decode(w, r, &in) {
			return
		}
		if s.SetPolicy(in.Hostname, in.Limits, in.Disabled, in.Revoked) != nil {
			http.Error(w, "policy refused", 403)
			return
		}
		w.WriteHeader(204)
	})
	return mux
}
func decode(w http.ResponseWriter, r *http.Request, out any) bool {
	r.Body = http.MaxBytesReader(w, r.Body, 4096)
	d := json.NewDecoder(r.Body)
	d.DisallowUnknownFields()
	if d.Decode(out) != nil {
		http.Error(w, "invalid request", 400)
		return false
	}
	return true
}
func reply(w http.ResponseWriter, out any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	json.NewEncoder(w).Encode(out)
}
