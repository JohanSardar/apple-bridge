package world

import (
	"context"
	"errors"
	"os/exec"
	"testing"
)

func TestCommandOutputDoesNotWaitForInheritedBuildPipes(t *testing.T) {
	_, err := commandOutput(context.Background(), "sh", "-c", "sleep 4 & exit 0")
	if !errors.Is(err, exec.ErrWaitDelay) {
		t.Fatalf("build descendants kept output pipes open without a bounded drain: %v", err)
	}
}
