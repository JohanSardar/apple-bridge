package world

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type CredentialResult struct {
	Backend string `json:"backend"`
	Created bool   `json:"created"`
	Used    bool   `json:"used"`
	Rotated bool   `json:"rotated"`
	Revoked bool   `json:"revoked"`
}

func CredentialLifecycle(ctx context.Context, home string) (CredentialResult, error) {
	store := trust.NewSecretStore(home)
	result := CredentialResult{Backend: store.Backend()}
	first, second := make([]byte, 32), make([]byte, 32)
	if _, err := rand.Read(first); err != nil {
		return result, err
	}
	if _, err := rand.Read(second); err != nil {
		return result, err
	}
	defer store.Delete(context.Background(), "lifecycle")
	if err := store.Put(ctx, "lifecycle", first); err != nil {
		return result, err
	}
	result.Created = true
	actual, err := store.Get(ctx, "lifecycle")
	if err != nil {
		return result, err
	}
	result.Used = bytes.Equal(first, actual)
	if err = store.Put(ctx, "lifecycle", second); err != nil {
		return result, err
	}
	actual, err = store.Get(ctx, "lifecycle")
	if err != nil {
		return result, err
	}
	result.Rotated = bytes.Equal(second, actual) && !bytes.Equal(first, actual)
	if err = store.Delete(ctx, "lifecycle"); err != nil {
		return result, err
	}
	_, err = store.Get(ctx, "lifecycle")
	result.Revoked = errors.Is(err, contracts.ErrNotFound)
	return result, nil
}
func (w *World) Credentials() (CredentialResult, error) {
	if w.Level != "replica" {
		return CredentialLifecycle(w.ctx, filepath.Join(w.Dir, "credentials"))
	}
	root, err := filepath.Abs("bin/bridge-lab-linux-amd64")
	if err != nil {
		return CredentialResult{}, err
	}
	binary := filepath.Join(w.Dir, "bridge-lab")
	if _, err = commandOutput(w.ctx, "scp", root, "ubuntu:"+binary); err != nil {
		return CredentialResult{}, err
	}
	output, err := w.execute(true, binary, "credential-lifecycle", "--home", filepath.Join(w.Dir, "credentials"))
	if err != nil {
		return CredentialResult{}, err
	}
	var result CredentialResult
	err = json.Unmarshal([]byte(output), &result)
	return result, err
}

func CleanupCredentials(ctx context.Context, home string) error {
	store := trust.NewSecretStore(home)
	if err := store.Delete(ctx, "identity-key"); err != nil {
		return err
	}
	if file, ok := store.(*trust.LinuxSecretStore); ok && !file.SecretTool {
		os.Remove(file.Dir)
	}
	return nil
}
