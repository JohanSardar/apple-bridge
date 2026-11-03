//go:build !lab

package iphone

import "net/http"

func observe(next http.Handler) http.Handler { return next }
