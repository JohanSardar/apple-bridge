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
	KeychainDefaultIsLogin *bool  `json:"keychain_default_is_login,omitempty"`
	KeychainUnlocked       *bool  `json:"keychain_unlocked,omitempty"`
	KeychainStatusCode     *int   `json:"keychain_status_code,omitempty"`
	State                  string `json:"state,omitempty"`
	Failure                string `json:"failure,omitempty"`
	Backend                string `json:"backend"`
	Created                bool   `json:"created"`
	Used                   bool   `json:"used"`
	Rotated                bool   `json:"rotated"`
	Revoked                bool   `json:"revoked"`
}

func CredentialLifecycle(ctx context.Context, home string) (CredentialResult, error) {
	return credentialLifecycle(ctx, trust.NewSecretStore(home))
}
func credentialLifecycle(ctx context.Context, store trust.SecretStore) (CredentialResult, error) {
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

func CredentialLifecycleGUI(ctx context.Context, home string) (CredentialResult, error) {
	defer os.RemoveAll(home)
	app := filepath.Join(home, "apps", "AppleBridgeHelper.app")
	root, err := os.Getwd()
	if err != nil {
		return CredentialResult{}, err
	}
	if _, err = commandOutput(ctx, filepath.Join(root, "packaging/dev/build-helper.sh"), app); err != nil {
		return CredentialResult{}, err
	}
	report := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev", filepath.Base(home)+".json")
	defer os.Remove(report)
	if _, err = commandOutput(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "probe", "keychain-state", "--report", report); err != nil {
		return CredentialResult{}, err
	}
	data, err := os.ReadFile(report)
	if err != nil {
		return CredentialResult{}, err
	}
	var state struct {
		Unlocked       bool `json:"unlocked"`
		StatusCode     int  `json:"status_code"`
		DefaultIsLogin bool `json:"default_is_login"`
	}
	if err = json.Unmarshal(data, &state); err != nil {
		return CredentialResult{}, err
	}
	result := CredentialResult{Backend: "keychain", KeychainUnlocked: &state.Unlocked, KeychainStatusCode: &state.StatusCode, KeychainDefaultIsLogin: &state.DefaultIsLogin, State: "unavailable"}
	if state.StatusCode != 0 || !state.DefaultIsLogin {
		result.Failure = "credential_store_unavailable"
		return result, nil
	}
	if !state.Unlocked {
		result.Failure = "credential_store_locked"
		return result, nil
	}
	absolute, err := filepath.Abs(home)
	if err != nil {
		return result, err
	}
	result, err = credentialLifecycle(ctx, &trust.KeychainStore{Service: "apple-bridge:" + trust.Fingerprint([]byte(absolute))})
	result.KeychainUnlocked = &state.Unlocked
	result.KeychainStatusCode = &state.StatusCode
	result.KeychainDefaultIsLogin = &state.DefaultIsLogin
	if err != nil {
		var problem *trust.CredentialStoreError
		if errors.As(err, &problem) {
			result.Failure = problem.Code
		}
	}
	return result, err
}
