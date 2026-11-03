//go:build !lab

package frontdoor

import "net/http"

func iphoneSecure(r *http.Request) bool { return r.TLS != nil }
