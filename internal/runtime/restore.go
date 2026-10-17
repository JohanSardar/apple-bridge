package runtime

import (
	"context"
	"os"
	"path/filepath"
	"syscall"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

func Restore(ctx context.Context, home, backup string, acceptLedgerLoss ...bool) error {
	if err := os.MkdirAll(home, 0700); err != nil {
		return err
	}
	lock, err := os.OpenFile(filepath.Join(home, "lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return contracts.ErrConflict
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	return state.Restore(ctx, home, backup, acceptLedgerLoss...)
}
