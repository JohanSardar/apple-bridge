package web

import (
	"context"
	"encoding/json"
	"errors"
	"io/fs"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	webui "github.com/JohanSardar/apple-bridge/web"
)

const SessionCookie = "__Host-ab_session"
const LocalSessionCookie = "ab_session_local"
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"

type Deps struct {
	RateClock    func() time.Time
	FrameWritten func(contracts.Cursor, time.Time)
	Operations   *operations.Service
	Authority    *trust.Authority
	Blobs        *blobs.Store
	PublicHost   func() string
	Address      string
	Assets       fs.FS
}

type server struct {
	deps        Deps
	mu          sync.Mutex
	rates       map[string]rate
	ceremonies  map[string]ceremony
	globalAuth  rate
	rateCleanup time.Time
	sockets     map[*liveSocket]bool
}
type rate struct {
	Failures  int
	FailureAt time.Time
	RetryAt   time.Time
	Minute    time.Time
	Day       time.Time
	Second    time.Time
	Minutes   int
	Days      int
	Seconds   int
}
type identity struct {
	Principal trust.Principal
	Session   trust.DeviceSession
	Token     string
}
type identityKey struct{}

func respond(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(value)
}
func reject(w http.ResponseWriter, status int, code string) {
	respond(w, status, map[string]any{"error": map[string]string{"code": code, "message": code}})
}

type servingKey struct{}
type serving struct {
	Host   string
	Scheme string
}

func servingOrigin(r *http.Request) string {
	value, _ := r.Context().Value(servingKey{}).(serving)
	return value.Scheme + "://" + value.Host
}
func rpID(r *http.Request) string {
	value, _ := r.Context().Value(servingKey{}).(serving)
	if host, _, err := net.SplitHostPort(value.Host); err == nil {
		return host
	}
	return value.Host
}
func (s *server) serving(r *http.Request) (serving, bool) {
	address := s.deps.Address
	if address == "" {
		if local, ok := r.Context().Value(http.LocalAddrContextKey).(net.Addr); ok {
			address = local.String()
		}
	}
	_, port, err := net.SplitHostPort(address)
	local := err == nil && port != "" && (r.Host == "localhost:"+port || r.Host == "127.0.0.1:"+port)
	public := s.deps.PublicHost != nil && s.deps.PublicHost() != "" && r.Host == s.deps.PublicHost() && r.TLS != nil
	if !local && !public {
		return serving{}, false
	}
	scheme := "http"
	if r.TLS != nil {
		scheme = "https"
	}
	return serving{Host: r.Host, Scheme: scheme}, true
}
func clientIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	ip, err := netip.ParseAddr(host)
	if err != nil {
		return "unknown"
	}
	ip = ip.Unmap()
	if ip.Is6() {
		return netip.PrefixFrom(ip, 64).Masked().String()
	}
	return ip.String()
}
func current(r *http.Request) identity {
	value, _ := r.Context().Value(identityKey{}).(identity)
	return value
}

func Handler(deps Deps) http.Handler {
	if deps.Assets == nil {
		deps.Assets = webui.Assets
	}
	s := &server{deps: deps, rates: map[string]rate{}, ceremonies: map[string]ceremony{}, sockets: map[*liveSocket]bool{}}
	mux := http.NewServeMux()
	for _, flow := range []string{"enroll", "login", "passkeys"} {
		mux.HandleFunc("POST /web/v1/"+flow+"/options", func(w http.ResponseWriter, r *http.Request) { s.authOptions(w, r, flow) })
		mux.HandleFunc("POST /web/v1/"+flow+"/finish", func(w http.ResponseWriter, r *http.Request) { s.authFinish(w, r, flow) })
	}
	mux.HandleFunc("POST /web/v1/handles", s.handles)
	mux.HandleFunc("GET /web/v1/session", func(w http.ResponseWriter, r *http.Request) { respond(w, 200, current(r).Session) })
	mux.HandleFunc("POST /web/v1/logout", func(w http.ResponseWriter, r *http.Request) {
		if err := s.deps.Authority.Revoke(current(r).Principal.ID); err != nil {
			reject(w, 503, "unavailable")
			return
		}
		setCookie(w, r, "", trust.DeviceSession{}, -1)
		respond(w, 200, map[string]bool{"ok": true})
	})
	mux.HandleFunc("GET /web/v1/devices", func(w http.ResponseWriter, r *http.Request) {
		devices, err := s.deps.Authority.DeviceSessions()
		if err != nil {
			reject(w, 503, "unavailable")
			return
		}
		respond(w, 200, map[string]any{"devices": devices})
	})
	mux.HandleFunc("DELETE /web/v1/devices/{id}", func(w http.ResponseWriter, r *http.Request) {
		id := r.PathValue("id")
		if !strings.HasPrefix(id, "device:") {
			reject(w, 404, "not_found")
			return
		}
		if s.deps.Authority.Revoke(id) != nil {
			reject(w, 404, "not_found")
			return
		}
		respond(w, 200, map[string]bool{"ok": true})
	})
	mux.HandleFunc("GET /web/v1/passkeys", func(w http.ResponseWriter, r *http.Request) {
		keys, err := s.deps.Authority.Passkeys(rpID(r))
		if err != nil {
			reject(w, 503, "unavailable")
			return
		}
		list := []map[string]any{}
		for _, k := range keys {
			list = append(list, map[string]any{"id": k.ID, "rp_id": k.RPID, "created_at": k.CreatedAt})
		}
		respond(w, 200, map[string]any{"passkeys": list})
	})
	mux.HandleFunc("DELETE /web/v1/passkeys/{id}", func(w http.ResponseWriter, r *http.Request) {
		if err := s.deps.Authority.DeletePasskey(rpID(r), r.PathValue("id")); err != nil {
			if errors.Is(err, contracts.ErrConflict) {
				reject(w, 409, "last_passkey")
			} else {
				reject(w, 404, "not_found")
			}
			return
		}
		respond(w, 200, map[string]bool{"ok": true})
	})
	mux.HandleFunc("GET /web/v1/live", s.live)
	mux.HandleFunc("GET /web/v1/blob/{digest}", s.blob)
	mux.HandleFunc("POST /web/v1/upload", s.upload)
	s.agentRoutes(mux)
	for _, route := range []string{"GET approvals", "POST approvals/{command_id}"} {
		method, path, _ := strings.Cut(route, " ")
		mux.HandleFunc(method+" /web/v1/"+path, func(w http.ResponseWriter, r *http.Request) { reject(w, 501, "not_implemented") })
	}
	mux.HandleFunc("/web/v1/", func(w http.ResponseWriter, r *http.Request) { reject(w, 404, "not_found") })
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "GET" && r.Method != "HEAD" {
			w.WriteHeader(405)
			return
		}
		s.static(w, r)
	})
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Security-Policy", CSP)
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Permissions-Policy", "camera=(self), microphone=(self), geolocation=(self)")
		if r.TLS != nil {
			w.Header().Set("Strict-Transport-Security", "max-age=31536000")
		}
		servingValue, allowed := s.serving(r)
		if !allowed {
			reject(w, 403, "host")
			return
		}
		r = r.WithContext(context.WithValue(r.Context(), servingKey{}, servingValue))

		if r.Method != "GET" && r.Method != "HEAD" || r.URL.Path == "/web/v1/live" {
			origins := r.Header.Values("Origin")
			if len(origins) != 1 || origins[0] != servingOrigin(r) {
				reject(w, 403, "origin")
				return
			}
		}
		if strings.HasPrefix(r.URL.Path, "/web/v1/") {
			w.Header().Set("Cache-Control", "no-store")
			public := strings.HasPrefix(r.URL.Path, "/web/v1/login/") || strings.HasPrefix(r.URL.Path, "/web/v1/enroll/")
			if public {
				ip := clientIP(r)
				if !s.allow("auth:"+ip, true) {
					w.Header().Set("Retry-After", "60")
					reject(w, 429, "rate_limited")
					return
				}
			} else {
				cookie, err := r.Cookie(sessionCookie(r))
				if err != nil {
					reject(w, 401, "unauthorized")
					return
				}
				p, session, err := deps.Authority.AuthenticateDevice(cookie.Value)
				if err != nil {
					reject(w, 401, "unauthorized")
					return
				}
				if !s.allow(p.ID, false) {
					w.Header().Set("Retry-After", "1")
					reject(w, 429, "rate_limited")
					return
				}
				if session.Kind == "trusted" {
					setCookie(w, r, cookie.Value, session, 0)
				}
				r = r.WithContext(context.WithValue(r.Context(), identityKey{}, identity{p, session, cookie.Value}))
			}
			if r.URL.Path != "/web/v1/upload" {
				r.Body = http.MaxBytesReader(w, r.Body, 1024*1024)
			}
		}
		mux.ServeHTTP(w, r)
	})
}

func (s *server) allow(key string, auth bool) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	now := s.rateTime()
	if _, exists := s.rates[key]; !exists && (len(s.rates) >= 8192 || auth && len(s.rates) >= 4096) {
		if now.Sub(s.rateCleanup) >= time.Minute {
			s.rateCleanup = now
			for id, entry := range s.rates {
				if now.Sub(entry.Day) >= 24*time.Hour {
					delete(s.rates, id)
				}
			}
		}
		if len(s.rates) >= 8192 || auth && len(s.rates) >= 4096 {
			if !auth {
				return false
			}
			oldest := ""
			var at time.Time
			for id, entry := range s.rates {
				if !strings.HasPrefix(id, "auth:") {
					continue
				}
				if oldest == "" || entry.Day.Before(at) {
					oldest = id
					at = entry.Day
				}
			}
			if oldest == "" {
				for id := range s.rates {
					oldest = id
					break
				}
			}
			delete(s.rates, oldest)
		}
	}
	r := s.rates[key]
	if auth {
		if now.Sub(s.globalAuth.Minute) >= time.Minute {
			s.globalAuth = rate{Minute: now}
		}
		s.globalAuth.Minutes++
		if s.globalAuth.Minutes > 600 {
			return false
		}
		if now.Sub(r.FailureAt) >= 15*time.Minute {
			r.Failures = 0
			r.RetryAt = time.Time{}
		}
	}
	if now.Sub(r.Minute) >= time.Minute {
		r.Minute = now
		r.Minutes = 0
	}
	if now.Sub(r.Day) >= 24*time.Hour {
		r.Day = now
		r.Days = 0
	}
	if now.Sub(r.Second) >= time.Second {
		r.Second = now
		r.Seconds = 0
	}
	r.Minutes++
	r.Days++
	r.Seconds++
	s.rates[key] = r
	if auth {
		return r.Minutes <= 10 && r.Days <= 100 && !now.Before(r.RetryAt)
	}
	return r.Seconds <= 50
}
func sessionCookie(r *http.Request) string {
	value, _ := r.Context().Value(servingKey{}).(serving)
	host, port, err := net.SplitHostPort(value.Host)
	if value.Scheme == "http" && r.TLS == nil && err == nil && port != "" && (host == "localhost" || host == "127.0.0.1") {
		return LocalSessionCookie
	}
	return SessionCookie
}
func setCookie(w http.ResponseWriter, r *http.Request, token string, session trust.DeviceSession, maxAge int) {
	cookie := &http.Cookie{Name: sessionCookie(r), Value: token, Path: "/", Secure: sessionCookie(r) == SessionCookie, HttpOnly: true, SameSite: http.SameSiteStrictMode, MaxAge: maxAge}
	if token != "" && session.Kind == "trusted" {
		cookie.MaxAge = max(1, int(time.Until(session.ExpiresAt).Seconds()))
		cookie.Expires = session.ExpiresAt
	}
	http.SetCookie(w, cookie)
}
func (s *server) static(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/")
	if strings.HasPrefix(path, "assets/") {
		w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
	} else if path == "sw.js" || path == "manifest.webmanifest" {
		w.Header().Set("Cache-Control", "no-cache")
	} else {
		path = "index.html"
		w.Header().Set("Cache-Control", "no-store")
	}
	data, err := fs.ReadFile(s.deps.Assets, "dist/"+path)
	if err != nil {
		if path == "index.html" {
			w.Header().Set("Content-Type", "text/html; charset=utf-8")
			w.Write([]byte("<!doctype html><html><body>Run make web to build Apple Bridge.</body></html>"))
			return
		}
		http.NotFound(w, r)
		return
	}
	http.ServeContent(w, r, path, time.Time{}, strings.NewReader(string(data)))
}

func (s *server) rateTime() time.Time {
	if s.deps.RateClock != nil {
		return s.deps.RateClock()
	}
	return time.Now()
}
