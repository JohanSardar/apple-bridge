package relay

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/iphone"
)

type IPhonePush struct {
	Hostname string        `json:"hostname"`
	Token    string        `json:"token"`
	Packet   iphone.Packet `json:"packet"`
	Expires  int64         `json:"expires"`
}

func (s *Server) iphonePush(w http.ResponseWriter, r *http.Request) {
	var request IPhonePush
	if !decode(w, r, &request) {
		return
	}
	token := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
	if !s.Store.authenticated(request.Hostname, token) {
		http.Error(w, "denied", 403)
		return
	}
	if s.IPhonePush == nil {
		http.Error(w, "push unavailable", 503)
		return
	}
	if request.Expires <= time.Now().Unix() || request.Expires > time.Now().Add(10*time.Minute).Unix() || len(request.Packet.EphemeralKey) != 32 || len(request.Packet.Signature) != 64 || len(request.Packet.Box) < 28 || len(request.Packet.Box) > 2800 {
		http.Error(w, "invalid", 400)
		return
	}
	s.mu.Lock()
	if s.pushRates == nil {
		s.pushRates = map[string]pushWindow{}
	}
	rate := s.pushRates[request.Hostname]
	if time.Since(rate.At) > time.Minute {
		rate = pushWindow{At: time.Now()}
	}
	rate.Count++
	s.pushRates[request.Hostname] = rate
	s.mu.Unlock()
	if rate.Count > 60 {
		http.Error(w, "rate limited", 429)
		return
	}
	raw, _ := json.Marshal(request.Packet)
	ctx, cancel := context.WithTimeout(r.Context(), 10*time.Second)
	defer cancel()
	if s.IPhonePush(ctx, request.Token, raw, time.Unix(request.Expires, 0)) != nil {
		http.Error(w, "push failed", 502)
		return
	}
	w.WriteHeader(204)
}

type pushWindow struct {
	At    time.Time
	Count int
}
