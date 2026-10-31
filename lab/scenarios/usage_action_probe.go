package scenarios

import (
	"context"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func runUsageActionProbe(ctx context.Context, s Scenario, level string) Evidence {
	var cli shortcutProbeCLI = shortcut.CLI{}
	qualified := false
	if level == "sim" {
		cli = &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Screen Time"}}, result: shortcut.Result{Data: []byte(`{"synthetic":true,"rows":1}`)}}
		qualified = true
	} else {
		file, err := os.Open("packaging/shortcuts/AB Screen Time.shortcut")
		if err == nil {
			var header [4]byte
			n, err := file.Read(header[:])
			file.Close()
			qualified = err == nil && n == 4 && string(header[:]) == "AEA1"
		}
	}
	return runShortcutProbe(ctx, s, level, "AB Screen Time", cli, qualified, func([]byte) bool { return true })
}
