package web

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"net/http"
)

func oauthScopes() []string {
	out := []string{}
	for _, v := range []string{"messages", "reminders", "calendar", "contacts", "notes", "usage"} {
		out = append(out, v+":read", v+":write")
	}
	return out
}
func (s *server) oauthRoutes(mux *http.ServeMux) {
	s.oauthAuthorizationRoutes(mux)
	s.oauthTokenRoute(mux)
	protected := func(w http.ResponseWriter, r *http.Request) {
		respond(w, 200, map[string]any{"resource": servingOrigin(r) + "/mcp", "authorization_servers": []string{servingOrigin(r)}, "scopes_supported": []string{"messages:read"}, "bearer_methods_supported": []string{"header"}})
	}
	mux.HandleFunc("GET /.well-known/oauth-protected-resource", protected)
	mux.HandleFunc("GET /.well-known/oauth-protected-resource/mcp", protected)
	mux.HandleFunc("GET /.well-known/oauth-authorization-server", func(w http.ResponseWriter, r *http.Request) {
		origin := servingOrigin(r)
		respond(w, 200, map[string]any{"issuer": origin, "authorization_endpoint": origin + "/oauth/authorize", "token_endpoint": origin + "/oauth/token", "registration_endpoint": origin + "/oauth/register", "response_types_supported": []string{"code"}, "grant_types_supported": []string{"authorization_code", "refresh_token"}, "token_endpoint_auth_methods_supported": []string{"none"}, "code_challenge_methods_supported": []string{"S256"}, "scopes_supported": oauthScopes(), "authorization_response_iss_parameter_supported": true})
	})
	mux.HandleFunc("POST /oauth/register", func(w http.ResponseWriter, r *http.Request) {
		var c trust.OAuthClient
		if json.NewDecoder(r.Body).Decode(&c) != nil {
			respond(w, 400, map[string]string{"error": "invalid_client_metadata"})
			return
		}
		c, err := s.deps.Authority.RegisterOAuth(c)
		if err != nil {
			respond(w, 400, map[string]string{"error": "invalid_client_metadata"})
			return
		}
		respond(w, 201, c)
	})
}
