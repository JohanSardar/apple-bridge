package main

import (
	"context"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func TestPublicEnrollmentUsesActiveOriginAndQR(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	home := t.TempDir()
	dir := filepath.Join(home, "remote")
	if err := os.Mkdir(dir, 0700); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(remote.Status{State: "on", URL: "https://public.example.test"})
	if err := os.WriteFile(filepath.Join(dir, "status.json"), raw, 0600); err != nil {
		t.Fatal(err)
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	previous := os.Stdout
	os.Stdout = writer
	defer func() { os.Stdout = previous }()
	output := make(chan string, 1)
	go func() { b, _ := io.ReadAll(reader); output <- string(b) }()
	err = run(context.Background(), []string{"web", "enroll", "--public", "--home", home})
	writer.Close()
	text := <-output
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(text, "https://public.example.test/enroll#") || strings.Contains(text, "localhost") || !strings.ContainsAny(text, "▀▄█") {
		t.Fatal("public URL or QR missing")
	}
	raw, _ = json.Marshal(remote.Status{State: "off"})
	os.WriteFile(filepath.Join(dir, "status.json"), raw, 0600)
	if err = run(context.Background(), []string{"web", "enroll", "--public", "--home", home}); err == nil {
		t.Fatal("public enrollment accepted Remote off")
	}
}
