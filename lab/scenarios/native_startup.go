package scenarios

import (
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func nativeStartupEvidence(w *world.World, e *Evidence, trace []nativeTrace) {
	var sent, committed float64
	checkpoint := ""
	for _, event := range trace {
		if event.Stage == "capture_sent" && event.Source == "reminders" {
			sent, checkpoint = event.WallNS, event.Checkpoint
			break
		}
	}
	for _, event := range trace {
		if checkpoint != "" && event.Stage == "committed_ack" && event.Checkpoint == checkpoint {
			committed = event.WallNS
			break
		}
	}
	pid, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	if err != nil || committed == 0 {
		return
	}
	script := `import ctypes,sys
class Info(ctypes.Structure):
 _fields_=[("ids",ctypes.c_uint32*12),("comm",ctypes.c_char*16),("name",ctypes.c_char*32),("tail",ctypes.c_uint32*6),("sec",ctypes.c_uint64),("usec",ctypes.c_uint64)]
i=Info();lib=ctypes.CDLL("/usr/lib/libproc.dylib");n=lib.proc_pidinfo(int(sys.argv[1]),3,0,ctypes.byref(i),ctypes.sizeof(i));assert n==ctypes.sizeof(i);print(i.sec*1000000000+i.usec*1000)`
	output, err := exec.Command("python3", "-c", script, strings.TrimSpace(string(pid))).Output()
	if err != nil {
		return
	}
	started, err := strconv.ParseFloat(strings.TrimSpace(string(output)), 64)
	if err != nil || started <= 0 || committed < started {
		return
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["first_reminders_checkpoint_after_start_ms"] = (committed - started) / 1e6
	e.Metrics["first_reminders_send_after_start_ms"] = (sent - started) / 1e6
	e.Metrics["startup_method"] = "OS process start to first source=reminders capture committed acknowledgment"
	helper, err := os.Open(filepath.Join(w.HelperApp, "Contents", "MacOS", "AppleBridgeHelper"))
	if err == nil {
		hash := sha256.New()
		_, copyErr := io.Copy(hash, helper)
		helper.Close()
		if copyErr == nil {
			e.Metrics["helper_binary_sha256"] = hex.EncodeToString(hash.Sum(nil))
		}
	}
}
