package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

const macClipboardSnapshot = `ObjC.import('AppKit');
function run() {
 const board = $.NSPasteboard.generalPasteboard;
 if (Number(board.accessBehavior) !== 2) throw Error("Clipboard requires Always Allow");
 const change = Number(board.changeCount);
 const items = board.pasteboardItems;
 const result = [];
 let size = 0;
 for (let i = 0; items && i < items.count; i++) {
  const item = items.objectAtIndex(i), saved = Object.create(null), types = item.types;
  const verifier = $.NSPasteboardItem.alloc.init;
  for (let j = 0; j < types.count; j++) {
   const type = types.objectAtIndex(j), data = item.dataForType(type);
   if (!data || ObjC.unwrap(data) == null) throw Error('clipboard data unavailable');
   size += Number(data.length);
   if (size > 16777216) throw Error('clipboard snapshot too large');
   if (!verifier.setDataForType(data, type)) throw Error('clipboard format cannot be restored');
   saved[ObjC.unwrap(type)] = ObjC.unwrap(data.base64EncodedStringWithOptions(0));
  }
  result.push(saved);
 }
 if (Number(board.changeCount) !== change) throw Error("clipboard changed during snapshot");
 return JSON.stringify(result);
}`

const macClipboardRestore = `ObjC.import('AppKit');
function run() {
 const input = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
 const saved = JSON.parse(ObjC.unwrap($.NSString.alloc.initWithDataEncoding(input, $.NSUTF8StringEncoding)));
 const items = [];
 for (const savedItem of saved) {
  const item = $.NSPasteboardItem.alloc.init;
  for (const type of Object.keys(savedItem)) {
   const data = $.NSData.alloc.initWithBase64EncodedStringOptions($(savedItem[type]), 0);
   if (!item.setDataForType(data, $(type))) throw Error('clipboard restore failed');
  }
  items.push(item);
 }
 const board = $.NSPasteboard.generalPasteboard;
 board.clearContents;
 if (items.length && !board.writeObjects($(items))) throw Error('clipboard restore failed');
 return 'restored';
}`

func macPasteboard(ctx context.Context, source string, input []byte) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "/usr/bin/osascript", "-l", "JavaScript", "-e", source)
	cmd.Stdin = bytes.NewReader(input)
	return cmd.Output()
}

func macClipboardCommands(ctx context.Context, client scripted.Client, w *world.World, real bool, add func(string, bool)) {
	if real {
		allowed := false
		ready := w.Await(20*time.Second, func() bool {
			var status contracts.ReadResult
			if client.Call(ctx, "sources.status", struct{}{}, &status) != nil {
				return false
			}
			for _, record := range status.Records {
				if record.Source != "mac" {
					continue
				}
				allowed = true
				for _, missing := range record.Coverage.Missing {
					if missing == "Clipboard requires Always Allow in Paste from Other Apps" {
						allowed = false
					}
				}
				return true
			}
			return false
		})
		add("clipboard_permission_status_ready", ready == nil)
		add("clipboard_helper_access_granted", ready == nil && allowed)
		if ready != nil || !allowed {
			return
		}
		saved, err := macPasteboard(ctx, macClipboardSnapshot, nil)
		var before []map[string]string
		valid := err == nil && json.Unmarshal(saved, &before) == nil
		add("clipboard_snapshot_saved", valid)
		if !valid {
			return
		}
		defer func() {
			cleanup, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			_, err := macPasteboard(cleanup, macClipboardRestore, saved)
			add("clipboard_restored", err == nil)
			restored, readErr := macPasteboard(cleanup, macClipboardSnapshot, nil)
			var after []map[string]string
			add("clipboard_restore_truth", readErr == nil && json.Unmarshal(restored, &after) == nil && reflect.DeepEqual(before, after))
		}()
	}
	tag := "[ab-test " + contracts.NewID("clipboard") + "]"
	args, _ := json.Marshal(map[string]any{"text": tag})
	var result contracts.CommandRecord
	err := client.Call(ctx, "mac.clipboard_write", contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &result)
	add("clipboard_write_readback", err == nil && result.State == contracts.CommandObserved && contracts.MacCompletionEvidence(result.Evidence, result.RequestHash))
	raw, _ := json.Marshal(result)
	add("clipboard_command_redacted", result.Command.Op == "mac.clipboard_write" && !bytes.Contains(raw, []byte(tag)) && string(result.Command.Args) == "{}")
	var query contracts.QueryResult
	err = client.Call(ctx, "mac.clipboard_read", contracts.Query{Args: json.RawMessage(`{}`)}, &query)
	var readback struct {
		Text string `json:"text"`
	}
	add("clipboard_read_matches", err == nil && json.Unmarshal(query.ResultJSON, &readback) == nil && readback.Text == tag)
	if real {
		snapshot, snapshotErr := macPasteboard(ctx, macClipboardSnapshot, nil)
		var items []map[string]string
		marked := false
		if snapshotErr == nil && json.Unmarshal(snapshot, &items) == nil && len(items) == 1 {
			_, transient := items[0]["org.nspasteboard.TransientType"]
			_, concealed := items[0]["org.nspasteboard.ConcealedType"]
			marked = transient && concealed
		}
		add("clipboard_manager_markers_present", marked)
	}
	private := true
	walkErr := filepath.WalkDir(w.Dir, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !d.Type().IsRegular() {
			return nil
		}
		data, err := os.ReadFile(path)
		if errors.Is(err, fs.ErrNotExist) {
			return nil
		}
		if err != nil {
			return err
		}
		if bytes.Contains(data, []byte(tag)) {
			private = false
		}
		return nil
	})
	add("clipboard_absent_from_durable_files", walkErr == nil && private)
}
