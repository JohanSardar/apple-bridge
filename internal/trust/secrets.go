package trust

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
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

const keychainScript = `import ctypes as c,json,sys
r=json.load(sys.stdin)
s=c.CDLL('/System/Library/Frameworks/Security.framework/Security')
p=c.c_void_p
u=c.c_uint32
s.SecKeychainSetUserInteractionAllowed.argtypes=[c.c_bool]
s.SecKeychainOpen.argtypes=[c.c_char_p,c.POINTER(p)]
s.SecKeychainFindGenericPassword.argtypes=[p,u,c.c_char_p,u,c.c_char_p,c.POINTER(u),c.POINTER(p),c.POINTER(p)]
s.SecKeychainAddGenericPassword.argtypes=[p,u,c.c_char_p,u,c.c_char_p,u,c.c_char_p,c.POINTER(p)]
s.SecKeychainItemModifyAttributesAndData.argtypes=[p,p,u,c.c_char_p]
s.SecKeychainItemDelete.argtypes=[p]
s.SecKeychainItemFreeContent.argtypes=[p,p]
code=s.SecKeychainSetUserInteractionAllowed(False)
k=p()
if code==0: code=s.SecKeychainOpen(r['path'].encode(),c.byref(k))
service=r['service'].encode()
account=r['key'].encode()
item=p()
length=u()
data=p()
opened=code==0
if opened: code=s.SecKeychainFindGenericPassword(k,len(service),service,len(account),account,c.byref(length),c.byref(data),c.byref(item))
value=''
if code==0 and r['op']=='get': value=c.string_at(data,length.value).decode()
if data: s.SecKeychainItemFreeContent(None,data)
if opened and r['op']=='put' and code in (0,-25300):
    value=r['value'].encode()
    if code==0: code=s.SecKeychainItemModifyAttributesAndData(item,None,len(value),value)
    else: code=s.SecKeychainAddGenericPassword(k,len(service),service,len(account),account,len(value),value,c.byref(item))
    value=''
if r['op']=='delete' and code==0: code=s.SecKeychainItemDelete(item)
print(json.dumps({'status':code,'value':value}))
`

func (s *KeychainStore) noninteractive(ctx context.Context, op, key string, value []byte) ([]byte, error) {
	if s.Path != "" {
		if err := s.unlock(ctx); err != nil {
			return nil, err
		}
	}
	path := s.Path
	if path == "" {
		path = filepath.Join(os.Getenv("HOME"), "Library", "Keychains", "login.keychain-db")
	}
	input, err := json.Marshal(map[string]string{"op": op, "path": path, "service": s.Service, "key": key, "value": hex.EncodeToString(value)})
	if err != nil {
		return nil, err
	}
	output, err := secretCommand(ctx, input, "python3", "-c", keychainScript)
	if err != nil {
		return nil, err
	}
	var result struct {
		Status *int   `json:"status"`
		Value  string `json:"value"`
	}
	if json.Unmarshal(output, &result) != nil || result.Status == nil {
		return nil, contracts.ErrCorrupt
	}
	switch *result.Status {
	case 0:
		if op == "get" {
			data, err := hex.DecodeString(result.Value)
			if err != nil {
				return nil, contracts.ErrCorrupt
			}
			return data, nil
		}
		return nil, nil
	case -25300:
		return nil, contracts.ErrNotFound
	case -25308:
		return nil, &CredentialStoreError{Code: "credential_store_locked", Store: "keychain", Remedy: "Unlock the login Keychain in an interactive macOS session, then retry bridge start.", Retryable: true}
	default:
		return nil, keychainFailure(fmt.Errorf("Security status %d", *result.Status))
	}
}
func (s *KeychainStore) Put(ctx context.Context, key string, value []byte) error {
	_, err := s.noninteractive(ctx, "put", key, value)
	return err
}
func (s *KeychainStore) Get(ctx context.Context, key string) ([]byte, error) {
	return s.noninteractive(ctx, "get", key, nil)
}
func (s *KeychainStore) Delete(ctx context.Context, key string) error {
	_, err := s.noninteractive(ctx, "delete", key, nil)
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
