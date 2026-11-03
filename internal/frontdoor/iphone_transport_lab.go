//go:build lab

package frontdoor

import (
	"net/http"
	"os"
)

func iphoneSecure(r *http.Request) bool {
	return r.TLS != nil || os.Getenv("APPLE_BRIDGE_LAB_IPHONE_PLAINTEXT") == "1"
}
