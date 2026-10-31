package scenarios

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/frontdoor"
	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func securityPublicHealth(add func(string, bool)) {
	handler := frontdoor.Handler(frontdoor.Deps{PublicHost: func() string { return "bridge.example" }}, nil)
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, httptest.NewRequest("GET", "https://bridge.example/v1/health", nil))
	add("public_health_requires_authentication", response.Code == 401)
}

func securityRemoteFailure(ctx context.Context, add func(string, bool)) {
	home, err := os.MkdirTemp("", "ab-test-security-failure-")
	if err != nil {
		add("remote_failure_fixture", false)
		return
	}
	defer os.RemoveAll(home)
	r := remote.NewWithOptions(home, remote.Options{Node: func(string, string) (remote.EmbeddedNode, error) {
		return nil, errors.New("https://login.example/secret?token=[ab-test]-credential")
	}})
	reader, writer, err := os.Pipe()
	if err != nil {
		add("remote_failure_pipe", false)
		return
	}
	previous := os.Stderr
	os.Stderr = writer
	defer func() { os.Stderr = previous }()
	running, cancel := context.WithCancel(ctx)
	done := make(chan struct{})
	go func() { defer close(done); r.Run(running, http.NotFoundHandler(), http.NotFoundHandler(), nil) }()
	bounded, stop := context.WithTimeout(ctx, 3*time.Second)
	var output bytes.Buffer
	err = remote.CommandWithOpener(bounded, home, "on", "embedded", "bridge", &output, func(context.Context, string) error { return errors.New("unexpected approval") })
	stop()
	cancel()
	<-done
	writer.Close()
	log, readErr := io.ReadAll(reader)
	reader.Close()
	safe := readErr == nil && err != nil && strings.Contains(err.Error(), "Remote setup failed") && !strings.Contains(err.Error(), "credential") && !bytes.Contains(log, []byte("credential")) && !bytes.Contains(log, []byte("https://"))
	add("remote_setup_failure_redacts_credential_urls_in_logs", safe)
}
