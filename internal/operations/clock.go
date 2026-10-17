//go:build !lab

package operations

import "time"

func gatewayNow() time.Time { return time.Now().UTC() }

func dropCommandResponse() bool { return false }
