package trust

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type CredentialStoreError struct {
	Code      string `json:"code"`
	Store     string `json:"store"`
	Remedy    string `json:"remedy"`
	Retryable bool   `json:"retryable"`
}

func (e *CredentialStoreError) Error() string {
	return fmt.Sprintf("%s: %s credential store unavailable; %s", e.Code, e.Store, e.Remedy)
}
func (e *CredentialStoreError) Unwrap() error { return contracts.ErrUnavailable }
func keychainFailure(err error) error {
	var status *exec.ExitError
	if errors.As(err, &status) && status.ExitCode() == 36 {
		return &CredentialStoreError{Code: "credential_store_locked", Store: "keychain", Remedy: "Unlock the login Keychain in an interactive macOS session, then retry bridge start.", Retryable: true}
	}
	return &CredentialStoreError{Code: "credential_store_unavailable", Store: "keychain", Remedy: "Check access to the login Keychain in an interactive macOS session, then retry bridge start.", Retryable: true}
}

type SecretStore interface {
	Put(context.Context, string, []byte) error
	Get(context.Context, string) ([]byte, error)
	Delete(context.Context, string) error
	Backend() string
}

type KeychainStore struct {
	Service string
	Path    string
}
type LinuxSecretStore struct {
	Dir        string
	Service    string
	SecretTool bool
}

func NewSecretStore(home string) SecretStore {
	absolute, _ := filepath.Abs(home)
	service := "apple-bridge:" + Fingerprint([]byte(absolute))
	if os.Getenv("APPLE_BRIDGE_SECRET_STORE") == "file" {
		return &LinuxSecretStore{Dir: filepath.Join(home, "secrets"), Service: service}
	}
	if runtime.GOOS == "darwin" {
		executable, _ := os.Executable()
		bundle := filepath.Dir(filepath.Dir(executable))
		if _, err := os.Stat(filepath.Join(bundle, "Library", "LaunchAgents", "dev.applebridge.gateway.plist")); err == nil {
			return &KeychainStore{Service: service, Path: filepath.Join(home, "Apple Bridge.keychain-db")}
		}
		return &KeychainStore{Service: service}
	}
	root := os.Getenv("XDG_CONFIG_HOME")
	if root == "" {
		root = filepath.Join(os.Getenv("HOME"), ".config")
	}
	_, err := exec.LookPath("secret-tool")
	return &LinuxSecretStore{Dir: filepath.Join(root, "apple-bridge", "secrets", Fingerprint([]byte(absolute))), Service: service, SecretTool: err == nil}
}
func secretCommand(ctx context.Context, input []byte, name string, args ...string) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	command := exec.CommandContext(ctx, name, args...)
	command.Stdin = bytes.NewReader(input)
	output, err := command.Output()
	if ctx.Err() != nil {
		if name == "security" {
			return nil, keychainFailure(ctx.Err())
		}
		return nil, contracts.ErrUnavailable
	}
	if err != nil {
		var status *exec.ExitError
		if errors.As(err, &status) && ((name == "security" && status.ExitCode() == 44) || (name == "secret-tool" && status.ExitCode() == 1)) {
			return nil, contracts.ErrNotFound
		}
		if name == "security" {
			return nil, keychainFailure(err)
		}
		return nil, contracts.ErrUnavailable
	}
	return bytes.TrimSpace(output), nil
}
func (s *KeychainStore) Backend() string { return "keychain" }
func (s *KeychainStore) Put(ctx context.Context, key string, value []byte) error {
	if err := s.unlock(ctx); err != nil {
		return err
	}
	args := []string{"add-generic-password", "-U", "-s", s.Service, "-a", key, "-w", hex.EncodeToString(value), "-T", "/usr/bin/security"}
	if s.Path != "" {
		args = append(args, s.Path)
	}
	_, err := secretCommand(ctx, nil, "security", args...)
	return err
}
func (s *KeychainStore) Get(ctx context.Context, key string) ([]byte, error) {
	if err := s.unlock(ctx); err != nil {
		return nil, err
	}
	args := []string{"find-generic-password", "-s", s.Service, "-a", key, "-w"}
	if s.Path != "" {
		args = append(args, s.Path)
	}
	output, err := secretCommand(ctx, nil, "security", args...)
	if err != nil {
		return nil, err
	}
	value, err := hex.DecodeString(string(output))
	if err != nil {
		return nil, contracts.ErrCorrupt
	}
	return value, nil
}
func (s *KeychainStore) Delete(ctx context.Context, key string) error {
	if err := s.unlock(ctx); err != nil {
		return err
	}
	args := []string{"delete-generic-password", "-s", s.Service, "-a", key}
	if s.Path != "" {
		args = append(args, s.Path)
	}
	_, err := secretCommand(ctx, nil, "security", args...)
	if errors.Is(err, contracts.ErrNotFound) {
		return nil
	}
	return err
}
func (s *LinuxSecretStore) Backend() string {
	if s.SecretTool {
		return "libsecret"
	}
	return "file_0600"
}
func (s *LinuxSecretStore) path(key string) string {
	return filepath.Join(s.Dir, Fingerprint([]byte(key)))
}
func (s *LinuxSecretStore) Put(ctx context.Context, key string, value []byte) error {
	if s.SecretTool {
		_, err := secretCommand(ctx, []byte(hex.EncodeToString(value)), "secret-tool", "store", "--label=Apple Bridge", "application", s.Service, "key", key)
		return err
	}
	if err := os.MkdirAll(s.Dir, 0700); err != nil {
		return err
	}
	if err := os.Chmod(s.Dir, 0700); err != nil {
		return err
	}
	file, err := os.CreateTemp(s.Dir, "secret-*")
	if err != nil {
		return err
	}
	defer os.Remove(file.Name())
	if _, err = file.Write(value); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if err = os.Rename(file.Name(), s.path(key)); err != nil {
		return err
	}
	dir, err := os.Open(s.Dir)
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func (s *LinuxSecretStore) Get(ctx context.Context, key string) ([]byte, error) {
	if s.SecretTool {
		output, err := secretCommand(ctx, nil, "secret-tool", "lookup", "application", s.Service, "key", key)
		if err != nil {
			return nil, err
		}
		return hex.DecodeString(strings.TrimSpace(string(output)))
	}
	info, err := os.Stat(s.path(key))
	if os.IsNotExist(err) {
		return nil, contracts.ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm() != 0600 {
		return nil, contracts.ErrUnauthorized
	}
	return os.ReadFile(s.path(key))
}
func (s *LinuxSecretStore) Delete(ctx context.Context, key string) error {
	if s.SecretTool {
		_, err := secretCommand(ctx, nil, "secret-tool", "clear", "application", s.Service, "key", key)
		return err
	}
	err := os.Remove(s.path(key))
	if os.IsNotExist(err) {
		return nil
	}
	return err
}

func (s *KeychainStore) unlock(ctx context.Context) error {
	if s.Path == "" {
		return nil
	}
	passwordPath := filepath.Join(filepath.Dir(s.Path), "keychain-password")
	password, err := os.ReadFile(passwordPath)
	if os.IsNotExist(err) {
		if _, statErr := os.Stat(s.Path); !os.IsNotExist(statErr) {
			return contracts.ErrUnauthorized
		}
		random := make([]byte, 32)
		if _, err = rand.Read(random); err != nil {
			return err
		}
		password = []byte(hex.EncodeToString(random))
		file, createErr := os.OpenFile(passwordPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if createErr != nil {
			return createErr
		}
		_, err = file.Write(password)
		if err == nil {
			err = file.Sync()
		}
		closeErr := file.Close()
		if err != nil {
			return err
		}
		if closeErr != nil {
			return closeErr
		}
	} else if err != nil {
		return err
	}
	info, err := os.Lstat(passwordPath)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm() != 0600 || len(password) != 64 {
		return contracts.ErrUnauthorized
	}
	if _, err = os.Stat(s.Path); os.IsNotExist(err) {
		search, searchErr := secretCommand(ctx, nil, "security", "list-keychains", "-d", "user")
		if searchErr != nil {
			return searchErr
		}
		defer func() {
			args := []string{"list-keychains", "-d", "user", "-s"}
			for _, line := range strings.Split(string(search), "\n") {
				if entry := strings.Trim(strings.TrimSpace(line), "\""); entry != "" {
					args = append(args, entry)
				}
			}
			secretCommand(context.Background(), nil, "security", args...)
		}()
		if _, err = secretCommand(ctx, nil, "security", "create-keychain", "-p", string(password), s.Path); err != nil {
			return err
		}
	}
	_, err = secretCommand(ctx, nil, "security", "unlock-keychain", "-p", string(password), s.Path)
	return err
}
