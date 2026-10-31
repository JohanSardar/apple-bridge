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
	mux.HandleFunc("POST /oauth/pending", func(w http.ResponseWriter, r *http.Request) {
		if !s.allow("connector-create:"+clientIP(r), true) {
			reject(w, 429, "Too many requests. Wait a minute and try again.")
			return
		}
		client, code, err := s.authorizationRequest(r)
		if err != nil {
			reject(w, 400, "This connection request is invalid. Start again from your connector.")
			return
		}
		pending, err := s.deps.ConnectorApprovals.begin(client, code, r.URL.Query().Get("state"), servingOrigin(r), toGrants(requestedScopes(r.URL.Query())))
		if err != nil {
			reject(w, 503, "Approval is unavailable. Try again in a few minutes.")
			return
		}
		respond(w, 201, pending)
	})
	mux.HandleFunc("GET /oauth/pending/{id}", func(w http.ResponseWriter, r *http.Request) {
		if !s.allowRate("connector-poll:"+clientIP(r), false, 10) {
			reject(w, 429, "Wait a moment and try again.")
			return
		}
		redirect, err := s.deps.ConnectorApprovals.poll(r.PathValue("id"), servingOrigin(r))
		if err != nil {
			reject(w, 410, "This approval has expired or was already used. Start again from your connector.")
			return
		}
		respond(w, 200, map[string]string{"redirect": redirect})
	})
	mux.HandleFunc("GET /oauth/authorize", func(w http.ResponseWriter, r *http.Request) {
		c, _, err := s.authorizationRequest(r)
		if err != nil {
			respond(w, 400, map[string]string{"error": "invalid_request"})
			return
		}
		if r.URL.Query().Get("format") == "json" {
			u, err := url.Parse(r.URL.Query().Get("redirect_uri"))
			if err != nil {
				respond(w, 400, map[string]string{"error": "invalid_request"})
				return
			}
			respond(w, 200, map[string]any{"client_name": c.Name, "redirect_host": u.Host, "scopes": requestedScopes(r.URL.Query()), "groups": contracts.GrantGroups(true)})
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
		if in.Level == "" {
			in.Level = trust.AgentApprove
		}
		grants, err := trust.AgentGrants(in.Level, in.Scopes)
		if in.Decision == "approve" && (err != nil || !trust.Subset(grants, toGrants(requestedScopes(r.URL.Query())))) {
			respond(w, 400, map[string]string{"error": "invalid_scope"})
			return
		}
		redirect, err := issueConnectorDecision(s.deps.Authority, c, code, r.URL.Query().Get("state"), servingOrigin(r), in.Decision, in.Level, grants)
		if err != nil {
			reject(w, 503, "unavailable")
			return
		}
		respond(w, 200, map[string]string{"redirect": redirect})
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
	var client trust.OAuthClient
	if strings.HasPrefix(q.Get("client_id"), "https://") {
		client, err = s.deps.ClientMetadata.Fetch(r.Context(), q.Get("client_id"), q.Get("redirect_uri"))
	} else {
		client, err = s.deps.Authority.OAuthClient(q.Get("client_id"), q.Get("redirect_uri"))
	}
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
