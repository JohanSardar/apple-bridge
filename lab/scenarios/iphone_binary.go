package scenarios

import (
	"context"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func iphoneBinaryFixture(ctx context.Context, dir string) (*phonefixture.Gateway, *simapple.IPhone, string, error) {
	selection := world.ImplementationSelection(ctx)
	goBinary, err := filepath.Abs("bin/bridge")
	if err != nil {
		return nil, nil, "", err
	}
	binary, err := selection.Binary(selection.Gateway, goBinary)
	if err != nil {
		return nil, nil, "", err
	}
	gateway, err := phonefixture.Start(ctx, binary, filepath.Join(dir, "home"))
	if err != nil {
		return nil, nil, "", err
	}
	failed := true
	defer func() {
		if failed {
			gateway.Close()
		}
	}()
	output, err := gateway.Command("pair", "--iphone", "--via-remote")
	if err != nil {
		return nil, nil, "", err
	}
	phone, err := simapple.NewIPhone()
	if err != nil {
		return nil, nil, "", err
	}
	status, err := phone.Pair(gateway.Client, gateway.URL, strings.Split(output, "\n")[0])
	if err != nil {
		return nil, nil, "", err
	}
	if status != 200 {
		return nil, nil, "", fmt.Errorf("phone pairing HTTP %d", status)
	}
	token, err := gateway.Command("token", "create", "--scope", "iphone:write")
	if err != nil {
		return nil, nil, "", err
	}
	failed = false
	return gateway, phone, token, nil
}
