//go:build !lab

package iphone

import "time"

func now() time.Time { return time.Now() }
