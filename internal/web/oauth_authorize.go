package web

import (
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"net/http"
	"net/url"
	"strings"
)

func (s *server) oauthAuthorizationRoutes(mux *http.ServeMux) {
	mux.HandleFunc("GET /oauth/authorize", func(w http.ResponseWriter, r *http.Request) {
		c, _, err := s.authorizationRequest(r)
		if err != nil {
			respond(w, 400, map[string]string{"error": "invalid_request"})
			return
		}
		if r.URL.Query().Get("format") == "json" {
			u, _ := url.Parse(r.URL.Query().Get("redirect_uri"))
			respond(w, 200, map[string]any{"client_name": c.Name, "redirect_host": u.Host, "scopes": requestedScopes(r.URL.Query())})
			return
		}
		s.static(w, r)
	})
	mux.HandleFunc("POST /oauth/authorize", func(w http.ResponseWriter, r *http.Request) {
		c, code, err := s.authorizationRequest(r)
		if err != nil {
			respond(w, 400, map[string]string{"error": "invalid_request"})
			return
		}
		var in struct {
			Level    trust.AgentLevel  `json:"level"`
			Scopes   []contracts.Grant `json:"scopes"`
			Decision string            `json:"decision"`
		}
		if json.NewDecoder(r.Body).Decode(&in) != nil || in.Decision != "approve" && in.Decision != "reject" {
			respond(w, 400, map[string]string{"error": "invalid_request"})
			return
		}
		dest, _ := url.Parse(code.RedirectURI)
		q := dest.Query()
		q.Set("state", r.URL.Query().Get("state"))
		q.Set("iss", servingOrigin(r))
		if in.Decision == "reject" {
			q.Set("error", "access_denied")
		} else {
			if in.Level == "" {
				in.Level = trust.AgentApprove
			}
			grants, err := trust.AgentGrants(in.Level, in.Scopes)
			if err != nil {
				respond(w, 400, map[string]string{"error": "invalid_scope"})
				return
			}
			requested := requestedScopes(r.URL.Query())
			ceiling := []contracts.Grant{}
			for _, v := range requested {
				ceiling = append(ceiling, contracts.Grant(v))
			}
			if !trust.Subset(grants, ceiling) {
				respond(w, 400, map[string]string{"error": "invalid_scope"})
				return
			}
			agent, _, err := s.deps.Authority.CreateAgent(c.Name, in.Level, grants)
			if err != nil {
				reject(w, 503, "unavailable")
				return
			}
			code.Principal = agent.ID
			raw, err := s.deps.Authority.IssueOAuthCode(code)
			if err != nil {
				s.deps.Authority.Revoke(agent.ID)
				reject(w, 503, "unavailable")
				return
			}
			q.Set("code", raw)
		}
		dest.RawQuery = q.Encode()
		respond(w, 200, map[string]string{"redirect": dest.String()})
	})
}

func (s *server) authorizationRequest(r *http.Request) (trust.OAuthClient, trust.OAuthCode, error) {
	q := r.URL.Query()
	empty := trust.OAuthClient{}
	code := trust.OAuthCode{}
	for _, v := range q {
		if len(v) != 1 {
			return empty, code, contracts.ErrInvalid
		}
	}
	challenge, err := base64.RawURLEncoding.DecodeString(q.Get("code_challenge"))
	if err != nil || len(challenge) != 32 || q.Get("code_challenge_method") != "S256" || q.Get("response_type") != "code" || q.Get("resource") != servingOrigin(r)+"/mcp" || len(q.Get("state")) > 2048 {
		return empty, code, contracts.ErrInvalid
	}
	if _, err = trust.AgentGrants(trust.AgentApprove, toGrants(requestedScopes(q))); err != nil {
		return empty, code, err
	}
	client, err := s.deps.Authority.OAuthClient(q.Get("client_id"), q.Get("redirect_uri"))
	if err != nil {
		return empty, code, err
	}
	return client, trust.OAuthCode{ClientID: client.ID, RedirectURI: q.Get("redirect_uri"), Challenge: q.Get("code_challenge"), Resource: q.Get("resource")}, nil
}
func toGrants(scopes []string) []contracts.Grant {
	out := []contracts.Grant{}
	for _, v := range scopes {
		out = append(out, contracts.Grant(v))
	}
	return out
}

func requestedScopes(q url.Values) []string {
	scopes := strings.Fields(q.Get("scope"))
	if len(scopes) == 0 {
		return []string{"messages:read"}
	}
	return scopes
}
