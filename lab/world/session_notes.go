package world

import (
	"bytes"
	"compress/gzip"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"unicode/utf16"
)

type sessionNote struct {
	Key      string
	Folder   string
	Title    string
	Text     string
	Locked   bool
	Pinned   bool
	Revision int
	Deleted  bool
}

type sessionFolder struct {
	Key     string
	Title   string
	Account string
	Deleted bool
}

type sessionNotes struct {
	Folders map[string]*sessionFolder
	Notes   map[string]*sessionNote
	Order   []string
}

func protoField(number int, wire int, payload []byte) []byte {
	out := []byte{byte(number<<3 | wire)}
	if wire == 2 {
		length := len(payload)
		for length >= 0x80 {
			out = append(out, byte(length)|0x80)
			length >>= 7
		}
		out = append(out, byte(length))
	}
	return append(out, payload...)
}

func protoVarint(number int, value int) []byte {
	out := []byte{byte(number<<3 | 0)}
	for value >= 0x80 {
		out = append(out, byte(value)|0x80)
		value >>= 7
	}
	return append(out, byte(value))
}

func notesBody(text string) []byte {
	units := len(utf16.Encode([]rune(text)))
	run := protoVarint(1, units)
	run = append(run, protoField(2, 2, append(protoVarint(3, 1), protoField(9, 2, make([]byte, 16))...))...)
	run = append(run, protoVarint(13, 1)...)
	note := protoField(2, 2, []byte(text))
	note = append(note, protoField(3, 2, protoVarint(2, units))...)
	note = append(note, protoField(5, 2, run)...)
	document := append(protoVarint(2, 0), protoField(3, 2, note)...)
	raw := protoField(2, 2, document)
	var packed bytes.Buffer
	writer := gzip.NewWriter(&packed)
	writer.Write(raw)
	writer.Close()
	return packed.Bytes()
}

func (w *World) stageSessionNotes(action any) error {
	var change struct {
		Key        string          `json:"key"`
		Collection string          `json:"collection"`
		Delete     bool            `json:"delete"`
		Data       json.RawMessage `json:"data"`
	}
	raw, err := json.Marshal(action)
	if err != nil || json.Unmarshal(raw, &change) != nil || change.Key == "" {
		return os.ErrInvalid
	}
	if change.Collection != "notes.note" && change.Collection != "notes.folder" {
		return nil
	}
	if w.sessionNotes == nil {
		w.sessionNotes = &sessionNotes{Folders: map[string]*sessionFolder{}, Notes: map[string]*sessionNote{}}
	}
	state := w.sessionNotes
	var data struct {
		Title    string `json:"title"`
		Account  string `json:"account"`
		Text     string `json:"plain_text"`
		Locked   bool   `json:"locked"`
		Pinned   bool   `json:"pinned"`
		FolderID struct {
			Ref struct {
				NativeKey string `json:"native_key"`
			} `json:"$ref"`
		} `json:"folder_id"`
	}
	json.Unmarshal(change.Data, &data)
	if change.Collection == "notes.folder" {
		folder := state.Folders[change.Key]
		if folder == nil {
			folder = &sessionFolder{Key: change.Key}
			state.Folders[change.Key] = folder
		}
		folder.Deleted = change.Delete
		if !change.Delete {
			folder.Title, folder.Account = data.Title, data.Account
		}
	} else {
		note := state.Notes[change.Key]
		if note == nil {
			note = &sessionNote{Key: change.Key}
			state.Notes[change.Key] = note
			state.Order = append(state.Order, change.Key)
		}
		note.Revision++
		note.Deleted = change.Delete
		if !change.Delete {
			note.Title, note.Text, note.Locked, note.Pinned = data.Title, data.Text, data.Locked, data.Pinned
			if data.FolderID.Ref.NativeKey != "" {
				note.Folder = data.FolderID.Ref.NativeKey
			}
		}
	}
	return w.writeSessionNotes(w.sessionRoots["notes"])
}

const notesSchema = "CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('lab-notes');" +
	"CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount');" +
	"CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);" +
	"CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT);"

func (w *World) writeSessionNotes(root string) error {
	state := w.sessionNotes
	accounts := map[string]int{}
	accountNames := []string{}
	folderKeys := make([]string, 0, len(state.Folders))
	for key, folder := range state.Folders {
		folderKeys = append(folderKeys, key)
		if _, ok := accounts[folder.Account]; !ok {
			accounts[folder.Account] = 0
			accountNames = append(accountNames, folder.Account)
		}
	}
	sort.Strings(folderKeys)
	sort.Strings(accountNames)
	rows := [][]any{}
	pk := 1
	for _, name := range accountNames {
		accounts[name] = pk
		rows = append(rows, []any{pk, 3, "account-" + name, nil, nil, name, nil, nil, nil, nil, 0, 0, 0, 0, 1})
		pk++
	}
	folderPK := map[string]int{}
	for _, key := range folderKeys {
		folder := state.Folders[key]
		folderPK[key] = pk
		deleted := 0
		if folder.Deleted {
			deleted = 1
		}
		rows = append(rows, []any{pk, 1, key, nil, folder.Title, nil, accounts[folder.Account], nil, nil, nil, deleted, 0, 0, 0, 1})
		pk++
	}
	bodies := [][]any{}
	for index, key := range state.Order {
		note := state.Notes[key]
		folder, ok := folderPK[note.Folder]
		if !ok && len(folderKeys) > 0 {
			folder, ok = folderPK[folderKeys[0]]
		}
		if !ok {
			continue
		}
		deleted, locked, pinned := 0, 0, 0
		if note.Deleted {
			deleted = 1
		}
		if note.Locked {
			locked = 1
		}
		if note.Pinned {
			pinned = 1
		}
		var body any
		if !note.Locked {
			body = base64.StdEncoding.EncodeToString(notesBody(note.Text))
		}
		bodies = append(bodies, []any{index + 1, body, note.Revision})
		rows = append(rows, []any{pk, 2, key, note.Title, nil, nil, nil, nil, folder, index + 1, deleted, 0, locked, pinned, note.Revision})
		pk++
	}
	encodedRows, err := json.Marshal(rows)
	if err != nil {
		return err
	}
	encodedBodies, err := json.Marshal(bodies)
	if err != nil {
		return err
	}
	if err = os.MkdirAll(root, 0700); err != nil {
		return err
	}
	temporary := filepath.Join(root, ".tmp-NoteStore.sqlite")
	os.Remove(temporary)
	script := "import base64,json,sqlite3,sys\nc=sqlite3.connect(sys.argv[1])\nc.executescript(sys.argv[2])\nc.executemany('INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZTITLE2,ZNAME,ZACCOUNT8,ZPARENT,ZFOLDER,ZNOTEDATA,ZMARKEDFORDELETION,ZFOLDERTYPE,ZISPASSWORDPROTECTED,ZISPINNED,Z_OPT) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',json.loads(sys.argv[3]))\nc.executemany('INSERT INTO ZICNOTEDATA VALUES(?,?,?)',[(a,base64.b64decode(b) if b else None,d) for a,b,d in json.loads(sys.argv[4])])\nc.commit()\nc.close()\n"
	if _, err = commandOutput(w.ctx, "python3", "-c", script, temporary, notesSchema, string(encodedRows), string(encodedBodies)); err != nil {
		return err
	}
	return os.Rename(temporary, filepath.Join(root, "NoteStore.sqlite"))
}
