package web

import (
	_ "embed"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"html/template"
	"net/http"
)

var page = template.Must(template.New("page").Parse(`<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Apple Bridge</title><link rel="stylesheet" href="/consent.css"><script src="/webauthn.js" defer></script></head><body><main><nav><a href="/">Apple Bridge</a> · <a href="/approvals">Approvals</a></nav><h1>{{.Title}}</h1>{{if .Client}}<p>{{.Client}} requests access to this bridge.</p>{{end}}{{if .Auth}}<button id="signin">Sign in with passkey</button><p id="message" role="status"></p>{{end}}{{if .Consent}}<form method="post"><label>Agent level <select name="level"><option value="read">Read</option><option value="approve" selected>Approve every command</option><option value="full">Full</option></select></label><fieldset><legend>Scopes</legend>{{range .Scopes}}<label><input type="checkbox" name="scopes" value="{{.}}" checked>{{.}}</label>{{end}}</fieldset><button name="decision" value="approve">Allow connection</button><button name="decision" value="reject">Reject</button></form>{{end}}{{range .Rows}}<article><h2>{{.operation}}</h2><p>{{.principal}} · {{.command_id}}</p><pre>{{.arguments}}</pre><details><summary>Owner arguments</summary><pre>{{.owner_arguments}}</pre></details><form method="post" action="/approvals/{{.command_id}}"><button name="decision" value="approve">Approve</button><button name="decision" value="reject">Reject</button></form></article>{{end}}{{if .Empty}}<p>No pending approvals.</p>{{end}}{{if .Redirect}}<p><a id="connector-redirect" href="{{.Redirect}}">Continue to connector</a></p>{{end}}{{if .Status}}<pre>{{.Status}}</pre>{{end}}{{if .Home}}<p>Consent and pending agent approvals. Manage agents with <code>bridge agent create</code>, <code>bridge agent list</code> and <code>bridge agent revoke</code>.</p>{{end}}</main></body></html>`))

func renderPage(w http.ResponseWriter, data any) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	page.Execute(w, data)
}
func (s *server) consent(w http.ResponseWriter, r *http.Request, client string) {
	renderPage(w, map[string]any{"Title": "Connect an agent", "Client": client, "Auth": current(r).Principal.ID == "", "Consent": true, "Scopes": requestedScopes(r.URL.Query())})
}
func (s *server) pageRoutes(mux *http.ServeMux) {
	mux.HandleFunc("GET /{$}", func(w http.ResponseWriter, r *http.Request) {
		renderPage(w, map[string]any{"Title": "Apple Bridge", "Home": true, "Auth": true})
	})
	mux.HandleFunc("GET /enroll", func(w http.ResponseWriter, r *http.Request) {
		renderPage(w, map[string]any{"Title": "Enroll first passkey", "Auth": true})
	})
	mux.HandleFunc("GET /webauthn.js", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/javascript; charset=utf-8")
		w.Write([]byte(webAuthnScript))
	})
	mux.HandleFunc("GET /consent.css", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/css")
		w.Write([]byte(`body{font:18px system-ui;line-height:1.6;margin:2rem auto;padding:0 1rem;max-width:48rem;color:#222;background:#fafafa}label{display:block}button,select{font:inherit;margin:.5rem;padding:.4rem}article,fieldset{border:1px solid #aaa;padding:1rem;margin:1rem 0}pre{white-space:pre-wrap;overflow-wrap:anywhere}a{color:#174b91}`))
	})
	mux.HandleFunc("GET /approvals", func(w http.ResponseWriter, r *http.Request) {
		if s.deps.Operations == nil {
			reject(w, 503, "unavailable")
			return
		}
		g, ok := s.deps.Operations.Target.(*operations.LocalGateway)
		if !ok {
			reject(w, 503, "unavailable")
			return
		}
		rows, err := g.Approvals(r.Context())
		if err != nil {
			reject(w, 503, "unavailable")
			return
		}
		out := []map[string]any{}
		for _, row := range rows {
			redacted, _ := json.MarshalIndent(state.RedactedArguments(row.Command.Args), "", "  ")
			full, _ := json.MarshalIndent(row.Command.Args, "", "  ")
			out = append(out, map[string]any{"command_id": row.Command.ID, "operation": row.Command.Op, "principal": row.Principal, "arguments": string(redacted), "owner_arguments": string(full)})
		}
		renderPage(w, map[string]any{"Title": "Pending approvals", "Rows": out, "Empty": len(out) == 0})
	})
	mux.HandleFunc("GET /approvals/{id}", func(w http.ResponseWriter, r *http.Request) {
		if s.deps.Operations == nil {
			reject(w, 503, "unavailable")
			return
		}
		g, ok := s.deps.Operations.Target.(*operations.LocalGateway)
		if !ok {
			reject(w, 503, "unavailable")
			return
		}
		record, err := g.State.Command(r.Context(), r.PathValue("id"))
		if err != nil {
			reject(w, 404, "not_found")
			return
		}
		status, _ := json.MarshalIndent(record, "", "  ")
		renderPage(w, map[string]any{"Title": "Command status", "Status": string(status)})
	})
	mux.HandleFunc("POST /approvals/{id}", func(w http.ResponseWriter, r *http.Request) {
		if r.ParseForm() != nil {
			reject(w, 400, "invalid")
			return
		}
		decision := r.PostForm.Get("decision")
		if decision != "approve" && decision != "reject" {
			reject(w, 400, "invalid")
			return
		}
		if s.deps.Operations == nil {
			reject(w, 503, "unavailable")
			return
		}
		g, ok := s.deps.Operations.Target.(*operations.LocalGateway)
		if !ok {
			reject(w, 503, "unavailable")
			return
		}
		record, err := g.DecideApproval(r.Context(), current(r).Principal, r.PathValue("id"), decision == "approve")
		if err != nil {
			reject(w, 409, "conflict")
			return
		}
		status, _ := json.MarshalIndent(record, "", "  ")
		renderPage(w, map[string]any{"Title": "Command status", "Status": string(status)})
	})
}

//go:embed assets/webauthn.js
var webAuthnScript string
