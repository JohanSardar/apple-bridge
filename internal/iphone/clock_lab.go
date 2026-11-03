//go:build lab

package iphone

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func now() time.Time { return contracts.DecisionNow() }
