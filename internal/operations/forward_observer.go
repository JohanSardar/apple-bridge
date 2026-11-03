//go:build !lab

package operations

import (
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func observeForward(header string, raw json.RawMessage) func(contracts.CommandResponse, error) {
	return nil
}
