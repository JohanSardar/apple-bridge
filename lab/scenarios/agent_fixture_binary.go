package scenarios

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type binaryAgentIdentity struct {
	ID     string   `json:"id"`
	Level  string   `json:"level"`
	Scopes []string `json:"scopes"`
}

type binaryAgentResult struct {
	Agent binaryAgentIdentity `json:"agent"`
	Token string              `json:"token"`
}

func binaryAgentToken(w *world.World, name, level, scope string) (string, error) {
	result, err := binaryCreateAgent(w, w.Gateway, name, level, scope)
	return result.Token, err
}

func binaryCreateAgent(w *world.World, endpoint world.Endpoint, name, level, scope string) (binaryAgentResult, error) {
	var credentials binaryAgentResult
	raw, err := w.EndpointCommand(endpoint, "agent", "create", "--name", name, "--level", level, "--scope", scope)
	if err != nil {
		return credentials, err
	}
	if err = json.Unmarshal(raw, &credentials); err != nil {
		return credentials, err
	}
	if credentials.Token == "" || credentials.Agent.ID == "" {
		return credentials, fmt.Errorf("binary agent token missing")
	}
	return credentials, nil
}
