package scenarios

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func binaryAgentToken(w *world.World, name, level, scope string) (string, error) {
	raw, err := w.GatewayCommand("agent", "create", "--name", name, "--level", level, "--scope", scope)
	if err != nil {
		return "", err
	}
	var credentials struct {
		Token string `json:"token"`
	}
	if err = json.Unmarshal(raw, &credentials); err != nil {
		return "", err
	}
	if credentials.Token == "" {
		return "", fmt.Errorf("binary agent token missing")
	}
	return credentials.Token, nil
}
