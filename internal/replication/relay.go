package replication

import (
	"net/http"

	"github.com/coder/websocket"
)

func (s *Server) RelayHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.TLS == nil || len(r.Header.Values("Origin")) != 0 {
			http.Error(w, "unauthorized", 401)
			return
		}
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		principal, certificate, err := s.Authority.RelayServer(r.Context(), conn)
		if err != nil {
			return
		}
		ctx, cancel := s.Authority.WatchPrincipal(r.Context(), principal)
		defer cancel()
		s.serve(ctx, conn, principal, certificate)
	})
}
