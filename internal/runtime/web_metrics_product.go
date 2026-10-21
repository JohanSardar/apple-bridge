//go:build !lab

package runtime

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"time"
)

func webCaptureCommitted(home string, after, through uint64, at time.Time) {}
func webFrameWritten(home string, cursor contracts.Cursor, at time.Time)   {}
