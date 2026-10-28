package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
)

func runSafariCapture(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	if s.ID == "SAF-IDLE" && level == "real" {
		return safariIdleReal(ctx, e)
	}

	dir, err := os.MkdirTemp("", "ab-safari-")
	add("fixture_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	fixture := exec.CommandContext(ctx, "python3", "-c", `
import sqlite3,plistlib,sys,time,datetime
p=sys.argv[1]
c=sqlite3.connect(p+'/History.db');c.executescript('PRAGMA journal_mode=WAL; CREATE TABLE history_items(id INTEGER PRIMARY KEY,url TEXT,visit_count INTEGER); CREATE TABLE history_visits(id INTEGER PRIMARY KEY,history_item INTEGER,visit_time REAL,title TEXT);')
n=time.time()-978307200
c.execute('INSERT INTO history_items VALUES(1,?,2)',('https://example.com/',));c.execute('INSERT INTO history_items VALUES(2,?,1)',('https://example.com/old',))
c.executemany('INSERT INTO history_visits VALUES(?,?,?,?)',[(1,1,n-50,'First'),(2,1,n,'Latest'),(3,2,n-100*86400,'Old')]);c.commit();c.close()
c=sqlite3.connect(p+'/CloudTabs.db');c.executescript("PRAGMA journal_mode=WAL; CREATE TABLE cloud_tab_devices(device_uuid TEXT,device_name TEXT); CREATE TABLE cloud_tabs(tab_uuid TEXT,device_uuid TEXT,title TEXT,url TEXT); INSERT INTO cloud_tab_devices VALUES('d','Synthetic'); INSERT INTO cloud_tabs VALUES('t','d','Synthetic','https://example.com/');");c.close()
def leaf(key):return dict(WebBookmarkType='WebBookmarkTypeLeaf',WebBookmarkUUID=key,URLString='https://example.com/',URIDictionary={'title':'Synthetic'})
a=leaf('a');b=leaf('b');c=leaf('c');c['ReadingList']={'DateAdded':datetime.datetime(2026,1,1),'DateLastViewed':datetime.datetime(2026,1,2)}
with open(p+'/Bookmarks.plist','wb') as f:plistlib.dump(dict(Children=[dict(WebBookmarkType='WebBookmarkTypeList',Title='Favorites',Children=[a]),dict(WebBookmarkType='WebBookmarkTypeList',Title='com.apple.ReadingList',Children=[b,c]),dict(WebBookmarkType='WebBookmarkTypeList',Title='Empty')]),f)
`, dir)
	_, err = fixture.CombinedOutput()
	add("fake_databases", err == nil)
	if err != nil {
		return e
	}
	build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=SafariChecks")
	build.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	build.Cancel = func() error { return syscall.Kill(-build.Process.Pid, syscall.SIGKILL) }
	output, err := build.CombinedOutput()
	add("native_build", err == nil)
	if err != nil {
		if ctx.Err() != nil {
			e.Assertions[len(e.Assertions)-1].Detail = "native build exceeded scenario deadline"
		}
		os.Stderr.Write(output)
		return e
	}
	path, err := os.ReadFile("native/.build/native-output-1")
	add("native_output", err == nil)
	if err != nil {
		return e
	}
	if level == "real" {
		return safariReal(ctx, s, e, filepath.Join(strings.TrimSpace(string(path)), "SafariChecks"))
	}

	timeout := 30 * time.Second
	if s.ID == "SAF-IDLE" {
		timeout = 6 * time.Minute
	}
	checkContext, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	output, err = exec.CommandContext(checkContext, filepath.Join(strings.TrimSpace(string(path)), "SafariChecks"), dir, s.ID).CombinedOutput()
	if s.ID == "SAF-IDLE" {
		fields := strings.Fields(string(output))
		cpu := 0.0
		validCPU := false
		if len(fields) == 4 {
			var parseErr error
			cpu, parseErr = strconv.ParseFloat(fields[3], 64)
			validCPU = parseErr == nil && !math.IsNaN(cpu) && !math.IsInf(cpu, 0) && cpu >= 0
		}
		add("source_cpu_under_two_percent_for_five_minutes", err == nil && validCPU && len(fields) == 4 && strings.Join(fields[:3], " ") == "Safari idle cpu" && cpu >= 0 && cpu < 2)
		if validCPU {
			e.Metrics = map[string]any{"source_cpu_percent": cpu, "idle_seconds": 300}
		}
		return e
	}
	var observations []contracts.Observation
	add("native_capture_fields_and_corruption_guard", err == nil && json.Unmarshal(output, &observations) == nil && len(observations) == 5)
	if !e.Passed {
		return e
	}
	return safariGateway(ctx, e, observations)
}
