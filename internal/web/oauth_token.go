package web

import "net/http"

func (s *server) oauthTokenRoute(mux *http.ServeMux) {
	mux.HandleFunc("POST /oauth/token", func(w http.ResponseWriter, r *http.Request) {
		if r.ParseForm() != nil || len(r.PostForm) == 0 {
			respond(w, 400, map[string]string{"error": "invalid_request"})
			return
		}
		for _, values := range r.PostForm {
			if len(values) != 1 {
				respond(w, 400, map[string]string{"error": "invalid_request"})
				return
			}
		}
		f := r.PostForm
		if f.Get("grant_type") != "authorization_code" && f.Get("grant_type") != "refresh_token" {
			respond(w, 400, map[string]string{"error": "unsupported_grant_type"})
			return
		}
		if f.Get("resource") != servingOrigin(r)+"/mcp" {
			respond(w, 400, map[string]string{"error": "invalid_target"})
			return
		}
		tokens, err := s.deps.Authority.ExchangeOAuth(f.Get("grant_type"), f.Get("client_id"), f.Get("code"), f.Get("redirect_uri"), f.Get("code_verifier"), f.Get("resource"), f.Get("refresh_token"), f.Get("scope"))
		if err != nil {
			respond(w, 400, map[string]string{"error": "invalid_grant"})
			return
		}
		respond(w, 200, tokens)
	})
}
