package frontdoor

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Deps struct {
	Blobs *blobs.Store

	Operations *operations.Service
	Authority  *trust.Authority
	PublicHost func() string
	Address    string
}
type window struct {
	At    time.Time
	Count int
}

func respond(w http.ResponseWriter, result any, err error) {
	w.Header().Set("Content-Type", "application/json")
	if err != nil {
		typed := contracts.ErrUnavailable
		var target *contracts.Error
		if errors.As(err, &target) {
			typed = target
		}
		status := 400
		switch typed.Code {
		case "forbidden":
			status = 403
		case "unauthorized":
			status = 401
		case "not_found":
			status = 404
		case "conflict", "reset", "id_reused":
			status = 409
		case "unavailable":
			status = 503
		case "capacity":
			status = 507
		case "unsupported":
			status = 422
		}
		w.WriteHeader(status)
		json.NewEncoder(w).Encode(map[string]any{"error": typed})
		return
	}
	json.NewEncoder(w).Encode(result)
}

func Handler(deps Deps, descriptors []contracts.OperationDescriptor) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/health", func(w http.ResponseWriter, r *http.Request) { respond(w, map[string]string{"state": "alive"}, nil) })
	for _, descriptor := range descriptors {
		name := descriptor.Name
		mux.HandleFunc("POST /v1/op/"+name, func(w http.ResponseWriter, r *http.Request) {
			p := r.Context().Value(principalKey{}).(trust.Principal)
			input, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 1024*1024))
			if err != nil {
				respond(w, nil, contracts.ErrInvalid)
				return
			}
			result, milliseconds, err := invokeTimed(r.Context(), deps, p, descriptor, input)
			if descriptor.ReadModel {
				w.Header().Set("Server-Timing", fmt.Sprintf("readmodel;dur=%.6f", milliseconds))
			}
			respond(w, result, err)
		})
	}
	mux.HandleFunc("POST /v1/op/{operation}", func(w http.ResponseWriter, r *http.Request) {
		if strings.HasPrefix(r.PathValue("operation"), "usage.") || r.PathValue("operation") == "safari.close_tab" {
			respond(w, nil, contracts.ErrUnsupported)
			return
		}
		http.NotFound(w, r)
	})
	mux.HandleFunc("GET /v1/records/{collection}/{id}", func(w http.ResponseWriter, r *http.Request) {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		result, err := deps.Operations.Read(r.Context(), p, "records.get", contracts.ReadQuery{Collection: r.PathValue("collection"), ID: r.PathValue("id")})
		respond(w, result, err)
	})
	mux.HandleFunc("POST /v1/blobs", func(w http.ResponseWriter, r *http.Request) {
		principal := r.Context().Value(principalKey{}).(trust.Principal)
		if !trust.Subset([]contracts.Grant{"messages:write"}, principal.Grants) || deps.Authority.Authorize(principal, "messages:write") != nil {
			respond(w, nil, contracts.ErrUnauthorized)
			return
		}
		if deps.Blobs == nil {
			respond(w, nil, contracts.ErrUnavailable)
			return
		}
		if _, local := deps.Operations.Target.(*operations.LocalGateway); !local {
			respond(w, nil, contracts.ErrUnsupported)
			return
		}
		id, err := deps.Blobs.StageMessageUpload(r.Context(), principal.ID, r.Body)
		respond(w, map[string]string{"upload_id": id}, err)
	})
	mux.HandleFunc("GET /v1/blobs/{digest}", func(w http.ResponseWriter, r *http.Request) {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		grant, err := deps.Operations.BlobGrant(r.Context(), p, r.URL.Query().Get("record"))
		if err != nil {
			respond(w, nil, err)
			return
		}
		if deps.Blobs == nil {
			respond(w, nil, contracts.ErrUnavailable)
			return
		}
		if strings.HasPrefix(r.URL.Query().Get("record"), "qry_") {
			if err := deps.Operations.EnsureQueryBlob(r.Context(), p, r.URL.Query().Get("record"), r.PathValue("digest")); err != nil {
				respond(w, nil, err)
				return
			}
		}
		f, err := deps.Blobs.OpenAuthorized(r.Context(), grant, r.PathValue("digest"))
		if err != nil {
			respond(w, nil, err)
			return
		}
		defer f.Close()
		w.Header().Set("Content-Type", "application/octet-stream")
		http.ServeContent(w, r, "", time.Time{}, f)
	})
	mux.HandleFunc("GET /v1/changes", func(w http.ResponseWriter, r *http.Request) {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		cursor := deps.Operations.Head()
		after := r.URL.Query().Get("after")
		if after == "" {
			after = r.Header.Get("Last-Event-ID")
		}
		if after != "" {
			parsed, err := contracts.ParseEventID(after)
			if err != nil {
				respond(w, nil, contracts.ErrReset)
				return
			}
			cursor = parsed
		}
		iterator, err := deps.Operations.Observe(r.Context(), p, cursor)
		if err != nil {
			respond(w, nil, err)
			return
		}
		controller := http.NewResponseController(w)
		write := func(event string) error {
			if err := controller.SetWriteDeadline(time.Now().Add(10 * time.Second)); err != nil {
				return err
			}
			if _, err := io.WriteString(w, event); err != nil {
				return err
			}
			return controller.Flush()
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.Header().Set("Cache-Control", "no-cache")
		if err := write(": connected\n\n"); err != nil {
			return
		}
		for {
			ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
			batch, err := iterator.Next(ctx)
			cancel()
			if errors.Is(err, context.DeadlineExceeded) {
				if err = write(": heartbeat\n\n"); err != nil {
					return
				}
				continue
			}
			if err != nil {
				if errors.Is(err, contracts.ErrReset) {
					write("event: reset\ndata: {\"error\":\"reset\"}\n\n")
				}
				return
			}
			b, err := json.Marshal(batch)
			if err != nil {
				return
			}
			cursor := contracts.Cursor{Epoch: batch.Epoch, SubscriptionRevision: batch.SubscriptionRevision, Sequence: batch.Through}
			if err = write(fmt.Sprintf("id: %s\nevent: batch\ndata: %s\n\n", cursor.EventID(), b)); err != nil {
				return
			}
		}
	})
	mux.Handle("/mcp", mcpHandler(deps, descriptors))
	mux.HandleFunc("GET /v1/openapi.json", func(w http.ResponseWriter, r *http.Request) { respond(w, openAPI(descriptors), nil) })
	var mu sync.Mutex
	rates := map[string]window{}
	_, port, _ := net.SplitHostPort(deps.Address)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		public := deps.PublicHost != nil
		hostAllowed := r.Host == "127.0.0.1:"+port || r.Host == "localhost:"+port
		if public {
			hostAllowed = deps.PublicHost() != "" && r.Host == deps.PublicHost()
		}
		if !hostAllowed {
			respond(w, nil, contracts.ErrUnauthorized)
			return
		}
		if _, exists := r.Header["Origin"]; exists {
			respond(w, nil, contracts.ErrUnauthorized)
			return
		}
		if r.URL.Path == "/v1/health" {
			mux.ServeHTTP(w, r)
			return
		}
		scheme := "http"
		if r.TLS != nil {
			scheme = "https"
		}
		w.Header().Set("WWW-Authenticate", fmt.Sprintf(`Bearer resource_metadata="%s://%s/.well-known/oauth-protected-resource"`, scheme, r.Host))
		header := r.Header.Get("Authorization")
		if !strings.HasPrefix(header, "Bearer ") {
			respond(w, nil, contracts.ErrUnauthorized)
			return
		}
		p, err := deps.Authority.AuthenticateToken(strings.TrimPrefix(header, "Bearer "))
		if err == nil && p.Resource != "" {
			scheme := "http"
			if r.TLS != nil {
				scheme = "https"
			}
			if p.Resource != scheme+"://"+r.Host+"/mcp" {
				err = contracts.ErrUnauthorized
			}
		}
		if err == nil && public && !p.RemoteAllowed() {
			err = contracts.ErrUnauthorized
		}
		if err != nil {
			respond(w, nil, err)
			return
		}
		w.Header().Del("WWW-Authenticate")
		pctx, cancel := deps.Authority.WatchPrincipal(r.Context(), p)
		defer cancel()
		r = r.WithContext(pctx)
		mu.Lock()
		rate := rates[p.ID]
		if time.Since(rate.At) > time.Second {
			rate = window{At: time.Now()}
		}
		rate.Count++
		rates[p.ID] = rate
		mu.Unlock()
		if rate.Count > 200 {
			http.Error(w, "rate limited", 429)
			return
		}
		if r.Method != "POST" || r.URL.Path != "/v1/blobs" {
			r.Body = http.MaxBytesReader(w, r.Body, 1024*1024)
		}
		mux.ServeHTTP(w, r.WithContext(context.WithValue(r.Context(), principalKey{}, p)))
	})
}

type principalKey struct{}
