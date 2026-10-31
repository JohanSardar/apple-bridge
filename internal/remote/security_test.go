package remote

import (
	"context"
	"errors"
	"io"
	"os"
	"strings"
	"testing"
)

func TestRemoteFailureDoesNotLogCredentialURL(t *testing.T) {
	r := New(t.TempDir())
	r.options.Node = func(string, string) (EmbeddedNode, error) {
		return nil, errors.New("https://login.example/secret?token=[ab-test]-credential")
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	previous := os.Stderr
	os.Stderr = writer
	defer func() { os.Stderr = previous }()
	r.serve(context.Background(), Config{ID: "[ab-test]", Backend: "embedded"})
	writer.Close()
	output, err := io.ReadAll(reader)
	reader.Close()
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(output), "credential") || strings.Contains(string(output), "https:") {
		t.Fatal("credential URL leaked to stderr")
	}
	var status Status
	if err = read(r.home, "status", &status); err != nil {
		t.Fatal(err)
	}
	if status.State != "error" || strings.Contains(status.LastError, "credential") {
		t.Fatal("unsafe remote failure status")
	}
}
