package world

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
)

type sessionRootsKey struct{}

func WithSessionRoot(ctx context.Context, source, root string) context.Context {
	roots, _ := ctx.Value(sessionRootsKey{}).(map[string]string)
	next := map[string]string{source: root}
	for key, value := range roots {
		next[key] = value
	}
	return context.WithValue(ctx, sessionRootsKey{}, next)
}

func SessionSources() []string {
	binary := os.Getenv("AB_LAB_SESSION")
	if binary == "" {
		return nil
	}
	sources := []string{}
	for _, source := range strings.Split(os.Getenv("AB_LAB_SESSION_SOURCES"), ",") {
		if source = strings.TrimSpace(source); source != "" {
			sources = append(sources, source)
		}
	}
	sort.Strings(sources)
	return sources
}

func (w *World) helperSocket(gateway string) string {
	if w.sessionSocket != "" {
		return w.sessionSocket
	}
	return gateway
}

func (w *World) startSession(ctx context.Context, gatewaySocket string) (string, error) {
	sources := SessionSources()
	if len(sources) == 0 {
		return gatewaySocket, nil
	}
	external, _ := ctx.Value(sessionRootsKey{}).(map[string]string)
	producers := map[string]string{}
	config := map[string]map[string]string{}
	w.sessionRoots = map[string]string{}
	for _, source := range sources {
		root := external[source]
		if root == "" {
			root = filepath.Join(w.Dir, "session-"+source)
			if err := os.MkdirAll(root, 0700); err != nil {
				return "", err
			}
		}
		producers[source] = "Rust"
		config[source] = map[string]string{"root": root}
		w.sessionRoots[source] = root
	}
	if _, ok := config["notes"]; ok {
		w.sessionNotesHold = filepath.Join(w.Dir, "session-notes-hold")
		config["notes"]["hydration_hold"] = w.sessionNotesHold
	}
	if _, ok := config["usage"]; ok {
		config["usage"]["local_name"] = "Synthetic mac local"
	}
	producerJSON, err := json.Marshal(producers)
	if err != nil {
		return "", err
	}
	configJSON, err := json.Marshal(config)
	if err != nil {
		return "", err
	}
	w.sessionState = filepath.Join(w.Dir, "session-state")
	w.sessionEnv = []string{"AB_SESSION_PRODUCERS=" + string(producerJSON), "AB_SESSION_RUST_SOURCES=" + string(configJSON), "AB_SESSION_RUST_STATE=" + w.sessionState}
	listen := filepath.Join(w.Dir, "session.sock")
	w.sessionListen, w.sessionGateway = listen, gatewaySocket
	if _, err = w.start("session", false, w.Dir, os.Getenv("AB_LAB_SESSION"), "--lab-listen", listen, "--lab-gateway", gatewaySocket); err != nil {
		return "", err
	}
	if err = w.Await(10*time.Second, func() bool { _, err := os.Stat(listen); return err == nil }); err != nil {
		return "", err
	}
	w.sessionSocket = listen
	return listen, nil
}

func (w *World) SessionCheckpoint(source string) string {
	raw, err := os.ReadFile(filepath.Join(w.sessionState, source+".json"))
	if err != nil {
		return ""
	}
	var record struct {
		Checkpoint string `json:"checkpoint"`
	}
	if json.Unmarshal(raw, &record) != nil {
		return ""
	}
	return record.Checkpoint
}

func (w *World) RestartSession() error {
	binary := os.Getenv("AB_LAB_SESSION")
	for _, p := range w.processes {
		if p != nil && !p.stopped && p.command.Path == binary {
			p.command.Process.Signal(syscall.SIGKILL)
			p.Stop()
		}
	}
	os.Remove(w.sessionListen)
	if _, err := w.start("session", false, w.Dir, binary, "--lab-listen", w.sessionListen, "--lab-gateway", w.sessionGateway); err != nil {
		return err
	}
	return w.Await(10*time.Second, func() bool { _, err := os.Stat(w.sessionListen); return err == nil })
}

type sessionDevice struct {
	Platform string
	Events   []sessionEvent
}

type sessionEvent struct {
	At       float64 `json:"at"`
	Starting bool    `json:"starting"`
	Bundle   string  `json:"bundle"`
}

func (w *World) stageSessionUsage(action any) error {
	var batch struct {
		Device   string         `json:"device"`
		Platform string         `json:"platform"`
		Events   []sessionEvent `json:"events"`
	}
	raw, err := json.Marshal(action)
	if err != nil || json.Unmarshal(raw, &batch) != nil || batch.Device == "" {
		return os.ErrInvalid
	}
	if w.sessionUsage == nil {
		w.sessionUsage = map[string]*sessionDevice{}
	}
	device := w.sessionUsage[batch.Device]
	if device == nil {
		device = &sessionDevice{Platform: batch.Platform}
		w.sessionUsage[batch.Device] = device
	}
	device.Events = append(device.Events, batch.Events...)
	root := w.sessionRoots["usage"]
	base := filepath.Join(root, "streams", "restricted", "App.InFocus")
	for id, device := range w.sessionUsage {
		directory := filepath.Join(base, "remote", id)
		if id == "local" {
			directory = filepath.Join(base, "local")
		}
		if err = os.MkdirAll(directory, 0700); err != nil {
			return err
		}
		if err = replaceFile(filepath.Join(directory, "1"), segb(device.Events)); err != nil {
			return err
		}
	}
	return w.writeSessionPeers(root)
}

func segb(events []sessionEvent) []byte {
	data := []byte("SEGB-lab")
	for _, event := range events {
		if event.Bundle == "" || len(event.Bundle) > 255 {
			continue
		}
		starting := byte(0)
		if event.Starting {
			starting = 1
		}
		data = append(data, 0x10, 1, 0x18, starting, 0x21)
		data = binary.LittleEndian.AppendUint64(data, math.Float64bits(event.At-978307200))
		data = append(data, 0x32, byte(len(event.Bundle)))
		data = append(data, event.Bundle...)
	}
	return data
}

func replaceFile(path string, data []byte) error {
	temporary := filepath.Join(filepath.Dir(path), ".tmp-"+filepath.Base(path))
	if err := os.WriteFile(temporary, data, 0600); err != nil {
		return err
	}
	return os.Rename(temporary, path)
}

func (w *World) writeSessionPeers(root string) error {
	rows := [][]any{}
	for id, device := range w.sessionUsage {
		platform, model, me := 2, "iPhone", 0
		switch device.Platform {
		case "mac":
			platform, model = 3, "Mac"
		case "ipad":
			model = "iPad"
		}
		if id == "local" {
			me = 1
		}
		rows = append(rows, []any{id, "Synthetic " + device.Platform + " " + id, platform, me, model})
	}
	encoded, err := json.Marshal(rows)
	if err != nil {
		return err
	}
	directory := filepath.Join(root, "sync")
	if err = os.MkdirAll(directory, 0700); err != nil {
		return err
	}
	temporary := filepath.Join(directory, ".tmp-sync.db")
	os.Remove(temporary)
	script := "import json,sqlite3,sys\nc=sqlite3.connect(sys.argv[1])\nc.execute('CREATE TABLE DevicePeer(device_identifier TEXT,name TEXT,platform INTEGER,me INTEGER,model TEXT)')\nc.executemany('INSERT INTO DevicePeer VALUES(?,?,?,?,?)',json.loads(sys.argv[2]))\nc.commit()\nc.close()\n"
	if _, err = commandOutput(w.ctx, "python3", "-c", script, temporary, string(encoded)); err != nil {
		return err
	}
	return os.Rename(temporary, filepath.Join(directory, "sync.db"))
}

type sessionMedia struct {
	file   string
	script string
}

var sessionMediaFixtures = map[string]sessionMedia{
	"phone": {
		file: "CallHistory.storedata",
		script: `import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-calls')")
c.execute("CREATE TABLE IF NOT EXISTS ZCALLRECORD(Z_PK INTEGER PRIMARY KEY,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER)")
c.execute("DELETE FROM ZCALLRECORD")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZCALLRECORD VALUES(1,'synthetic','Fixture',946684800.0,?,0,1,'com.apple.Telephony',1)",(100+int(sys.argv[2]),))
c.commit();c.close()
`,
	},
	"voicemail": {
		file: "voicemail.db",
		script: `import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-voicemail')")
c.execute("CREATE TABLE IF NOT EXISTS ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER)")
c.execute("DELETE FROM ZSTOREDMESSAGE")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,'lab-voicemail-1','Synthetic',946684800.0,?,0,0,'com.apple.coretelephony',1)",(100+int(sys.argv[2]),))
c.commit();c.close()
`,
	},
	"voice_memos": {
		file: "CloudRecordings.db",
		script: `import sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT)")
if not c.execute("SELECT COUNT(*) FROM Z_METADATA").fetchone()[0]:c.execute("INSERT INTO Z_METADATA VALUES('lab-recordings')")
c.execute("CREATE TABLE IF NOT EXISTS ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER)")
c.execute("DELETE FROM ZCLOUDRECORDING")
if sys.argv[2]!="absent":c.execute("INSERT INTO ZCLOUDRECORDING VALUES(1,'lab-recording-1','Synthetic',946684800.0,?,NULL,1)",(100+int(sys.argv[2]),))
c.commit();c.close()
`,
	},
}

func SessionMediaSource(source string) bool {
	if _, ok := sessionMediaFixtures[source]; !ok {
		return false
	}
	for _, candidate := range SessionSources() {
		if candidate == source {
			return true
		}
	}
	return false
}

func SessionMediaVersion(duration float64) int {
	return int(duration) - 100
}

func (w *World) StageSessionMedia(source string, version int, present bool) error {
	fixture, ok := sessionMediaFixtures[source]
	root := w.sessionRoots[source]
	if !ok || root == "" {
		return os.ErrInvalid
	}
	if os.Getenv("AB_LAB_SESSION_CONTROL") == "empty" {
		return nil
	}
	state := "absent"
	if present {
		state = strconv.Itoa(version)
	}
	_, err := commandOutput(w.ctx, "python3", "-c", fixture.script, filepath.Join(root, fixture.file), state)
	return err
}
