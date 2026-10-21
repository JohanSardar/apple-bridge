package demo

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

const SessionCookie = "ab_demo_session"

type Server struct {
	URL    string
	server *http.Server
}

func Serve(w *world.World, listen string) (*Server, error) {
	host, _, err := net.SplitHostPort(listen)
	if err != nil {
		return nil, err
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return nil, fmt.Errorf("demo listener must be a loopback IP")
	}
	listener, err := net.Listen("tcp", listen)
	if err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			listener.Close()
		}
	}()
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return nil, err
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	secret := make([]byte, 32)
	if _, err = rand.Read(secret); err != nil {
		return nil, err
	}
	path := "/__demo/" + base64.RawURLEncoding.EncodeToString(secret)
	target, err := url.Parse(w.Gateway.URL)
	if err != nil {
		return nil, err
	}
	proxy := httputil.NewSingleHostReverseProxy(target)
	director := proxy.Director
	proxy.Director = func(request *http.Request) {
		externalOrigin := "http://" + request.Host
		director(request)
		request.Host = target.Host
		cookies := request.Cookies()
		request.Header.Del("Cookie")
		for _, cookie := range cookies {
			if cookie.Name == bridgeweb.LocalSessionCookie {
				continue
			}
			if cookie.Name == SessionCookie {
				cookie.Name = bridgeweb.LocalSessionCookie
			}
			request.AddCookie(cookie)
		}
		if request.Header.Get("Origin") == externalOrigin {
			request.Header.Set("Origin", target.String())
		}
	}
	proxy.ModifyResponse = func(response *http.Response) error {
		cookies := response.Cookies()
		response.Header.Del("Set-Cookie")
		for _, cookie := range cookies {
			if cookie.Name == bridgeweb.LocalSessionCookie {
				cookie.Name = SessionCookie
				cookie.Secure = false
			}
			response.Header.Add("Set-Cookie", cookie.String())
		}
		return nil
	}
	var mu sync.Mutex
	used := false
	mux := http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Referrer-Policy", "no-referrer")
		_, port, _ := net.SplitHostPort(listener.Addr().String())
		if request.Host != "localhost:"+port && request.Host != "127.0.0.1:"+port {
			http.Error(response, "invalid host", http.StatusForbidden)
			return
		}
		if request.URL.Path == path {
			mu.Lock()
			fresh := !used && request.Method == "GET"
			if fresh {
				used = true
			}
			mu.Unlock()
			if !fresh {
				http.Error(response, "demo link already used", http.StatusGone)
				return
			}
			http.SetCookie(response, &http.Cookie{Name: SessionCookie, Value: token, Path: "/", Secure: false, HttpOnly: true, SameSite: http.SameSiteStrictMode, MaxAge: 86400})
			response.Header().Set("Cache-Control", "no-store")
			http.Redirect(response, request, "/messages", http.StatusSeeOther)
			return
		}
		if request.Method != "GET" && request.Method != "HEAD" || request.URL.Path == "/web/v1/live" {
			origins := request.Header.Values("Origin")
			if len(origins) != 1 || origins[0] != "http://"+request.Host {
				http.Error(response, "invalid origin", http.StatusForbidden)
				return
			}
		}
		proxy.ServeHTTP(response, request)
	})
	server := &http.Server{Handler: mux, ReadHeaderTimeout: 5 * time.Second}
	go server.Serve(listener)
	_, port, _ := net.SplitHostPort(listener.Addr().String())
	success = true
	return &Server{URL: "http://localhost:" + port + path, server: server}, nil
}

func (s *Server) Close() error {
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	return s.server.Shutdown(ctx)
}
