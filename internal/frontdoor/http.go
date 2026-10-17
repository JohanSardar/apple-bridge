package frontdoor

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Deps struct {
	Operations *operations.Service
	Authority  *trust.Authority
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
		case "unauthorized":
			status = 401
		case "not_found":
			status = 404
		case "conflict", "reset":
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
			var q contracts.ReadQuery
			decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 1024*1024))
			decoder.DisallowUnknownFields()
			if err := decoder.Decode(&q); err != nil {
				respond(w, nil, contracts.ErrInvalid)
				return
			}
			if decoder.Decode(&struct{}{}) != io.EOF {
				respond(w, nil, contracts.ErrInvalid)
				return
			}
			result, err := deps.Operations.Read(r.Context(), p, name, q)
			if name == "sources.status" && err == nil && deps.Operations.LocalStatus != nil {
				respond(w, map[string]any{"records": result.Records, "cursor": result.Cursor, "replica": deps.Operations.LocalStatus()}, nil)
				return
			}
			respond(w, result, err)
		})
	}
	mux.HandleFunc("GET /v1/records/{collection}/{id}", func(w http.ResponseWriter, r *http.Request) {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		result, err := deps.Operations.Read(r.Context(), p, "records.get", contracts.ReadQuery{Collection: r.PathValue("collection"), ID: r.PathValue("id")})
		respond(w, result, err)
	})
	mux.HandleFunc("GET /v1/changes", func(w http.ResponseWriter, r *http.Request) {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		cursor := deps.Operations.Head()
		after := r.URL.Query().Get("after")
		if after == "" {
			after = r.Header.Get("Last-Event-ID")
		}
		if after != "" {
			n, err := strconv.ParseUint(after, 10, 64)
			if err != nil {
				respond(w, nil, contracts.ErrInvalid)
				return
			}
			cursor.Sequence = n
		}
		if epoch := r.URL.Query().Get("epoch"); epoch != "" {
			cursor.Epoch = epoch
		}
		iterator, err := deps.Operations.Observe(r.Context(), p, cursor)
		if err != nil {
			respond(w, nil, err)
			return
		}
		flusher, ok := w.(http.Flusher)
		if !ok {
			respond(w, nil, contracts.ErrUnsupported)
			return
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.Header().Set("Cache-Control", "no-cache")
		w.WriteHeader(200)
		flusher.Flush()
		for {
			ctx, cancel := context.WithTimeout(r.Context(), 5*time.Second)
			batch, err := iterator.Next(ctx)
			cancel()
			if errors.Is(err, context.DeadlineExceeded) {
				if _, err = fmt.Fprint(w, ": heartbeat\n\n"); err != nil {
					return
				}
				flusher.Flush()
				continue
			}
			if err != nil {
				return
			}
			b, err := json.Marshal(batch)
			if err != nil {
				return
			}
			http.NewResponseController(w).SetWriteDeadline(time.Now().Add(10 * time.Second))
			if _, err = fmt.Fprintf(w, "id: %d\nevent: batch\ndata: %s\n\n", batch.Through, b); err != nil {
				return
			}
			flusher.Flush()
		}
	})
	var mu sync.Mutex
	rates := map[string]window{}
	_, port, _ := net.SplitHostPort(deps.Address)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != "127.0.0.1:"+port && r.Host != "localhost:"+port {
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
		header := r.Header.Get("Authorization")
		if !strings.HasPrefix(header, "Bearer ") {
			respond(w, nil, contracts.ErrUnauthorized)
			return
		}
		p, err := deps.Authority.AuthenticateToken(strings.TrimPrefix(header, "Bearer "))
		if err != nil {
			respond(w, nil, err)
			return
		}
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
		mux.ServeHTTP(w, r.WithContext(context.WithValue(r.Context(), principalKey{}, p)))
	})
}

type principalKey struct{}
