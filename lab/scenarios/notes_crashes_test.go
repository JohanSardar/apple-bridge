package scenarios

import (
	"encoding/binary"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestNotesHelperBinaryIdentityComesFromMachO(t *testing.T) {
	app := t.TempDir()
	path := filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper")
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	data := []byte{}
	for _, value := range []uint32{0xfeedfacf, 0x100000c, 0, 2, 1, 24, 0, 0, 0x1b, 24} {
		data = binary.LittleEndian.AppendUint32(data, value)
	}
	for i := byte(0); i < 16; i++ {
		data = append(data, i)
	}
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	e := Evidence{}
	notesRecordHelperBinary(&e, app)
	notesRecordHelperBinary(&e, app)
	if !reflect.DeepEqual(e.Metrics["notes_helper_binary_uuids"], []string{"00010203-0405-0607-0809-0a0b0c0d0e0f"}) {
		t.Fatalf("wrong Helper identity: %v", e.Metrics)
	}
}

func TestNotesCrashesMatchTheBinaryUsedByTheRun(t *testing.T) {
	for _, test := range []struct {
		name   string
		report string
		passed bool
	}{
		{"matching binary", "{}\n" + `{"usedImages":[{"name":"AppleBridgeHelper","uuid":"00010203-0405-0607-0809-0A0B0C0D0E0F"}]}`, false},
		{"different worker binary", "{}\n" + `{"usedImages":[{"name":"AppleBridgeHelper","uuid":"10010203-0405-0607-0809-0a0b0c0d0e0f"}]}`, true},
		{"unreadable report", "{incomplete", false},
		{"missing binary identity", "{}\n{}", false},
		{"invalid binary identity", "{}\n" + `{"usedImages":[{"name":"AppleBridgeHelper","uuid":"invalid"}]}`, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.WriteFile(filepath.Join(dir, "AppleBridgeHelper-test.ips"), []byte(test.report), 0600); err != nil {
				t.Fatal(err)
			}
			e := Evidence{Passed: true}
			notesAddHelperBinary(&e, "00010203-0405-0607-0809-0a0b0c0d0e0f")
			result := failOnNotesHelperCrashes(e, dir, nil)
			if result.Passed != test.passed {
				t.Fatalf("passed=%t, want %t: %+v", result.Passed, test.passed, result.Assertions)
			}
			unidentified := failOnNotesHelperCrashes(Evidence{Passed: true}, dir, nil)
			if unidentified.Passed {
				t.Fatal("run without a captured binary identity ignored a crash")
			}
		})
	}
}
