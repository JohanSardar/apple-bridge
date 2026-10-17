package world

import (
	"context"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (w *World) CappedGatewayDisk() error {
	w.StopGateway()
	image := filepath.Join(w.Dir, "capacity.sparseimage")
	if _, err := commandOutput(w.ctx, "hdiutil", "create", "-size", "64m", "-fs", "HFS+", "-volname", "BridgeLab", "-type", "SPARSE", image); err != nil {
		return err
	}
	mount := filepath.Join(w.Dir, "disk")
	if err := os.Mkdir(mount, 0700); err != nil {
		return err
	}
	if _, err := commandOutput(w.ctx, "hdiutil", "attach", image, "-mountpoint", mount, "-nobrowse"); err != nil {
		return err
	}
	w.diskMount = mount
	home := filepath.Join(mount, "gateway")
	if _, err := commandOutput(w.ctx, "cp", "-R", w.Gateway.Home, home); err != nil {
		return err
	}
	key, err := trust.NewSecretStore(w.Gateway.Home).Get(w.ctx, "identity-key")
	if err != nil {
		return err
	}
	if err = trust.NewSecretStore(home).Put(w.ctx, "identity-key", key); err != nil {
		return err
	}
	w.secretHomes = append(w.secretHomes, w.Gateway.Home)
	w.Gateway.Home = home
	return w.RestartGateway(0)
}
func (w *World) FillDisk() error {
	file, err := os.OpenFile(filepath.Join(w.diskMount, "filler"), os.O_CREATE|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	defer file.Close()
	block := make([]byte, 1024*1024)
	for {
		if _, err = file.Write(block); err != nil {
			break
		}
	}
	return file.Sync()
}
func (w *World) FreeDisk() error { return os.Remove(filepath.Join(w.diskMount, "filler")) }
func (w *World) CleanupCredential(home string) {
	trust.NewSecretStore(home).Delete(context.Background(), "identity-key")
}
