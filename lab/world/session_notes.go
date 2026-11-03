package world

import (
	"bytes"
	"compress/gzip"
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
	"unicode/utf16"

	"golang.org/x/net/html"
)

type sessionNotes struct {
	mu        sync.Mutex
	signature string
	folderPK  map[string]int
	notePK    map[string]int
	accountPK map[string]int
	revisions map[string]int
	versions  map[string]string
	stop      chan struct{}
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

type notesRun struct {
	Text      string
	Weight    int
	Underline bool
	Strike    bool
	Attach    string
}

type notesParagraph struct {
	Style   int
	Checked bool
	ID      []byte
	Runs    []notesRun
}

const hashtagUTI = "com.apple.notes.inlinetextattachment.hashtag"

func notesRunMessage(run notesRun, paragraph *notesParagraph) []byte {
	units := len(utf16.Encode([]rune(run.Text)))
	message := protoVarint(1, units)
	if paragraph != nil {
		inner := []byte{}
		if paragraph.Style >= 0 {
			inner = append(inner, protoVarint(1, paragraph.Style)...)
		}
		inner = append(inner, protoVarint(3, 1)...)
		if paragraph.Style == 103 {
			inner = append(inner, protoField(5, 2, append(protoField(1, 2, paragraph.ID), protoVarint(2, flag(paragraph.Checked))...))...)
		}
		inner = append(inner, protoField(9, 2, make([]byte, 16))...)
		message = append(message, protoField(2, 2, inner)...)
	}
	if run.Weight > 0 {
		message = append(message, protoVarint(5, run.Weight)...)
	}
	if run.Underline {
		message = append(message, protoVarint(6, 1)...)
	}
	if run.Strike {
		message = append(message, protoVarint(7, 1)...)
	}
	if run.Attach != "" {
		message = append(message, protoField(12, 2, append(protoField(1, 2, []byte(run.Attach)), protoField(2, 2, []byte(hashtagUTI))...))...)
	}
	return append(message, protoVarint(13, 1)...)
}

func notesBodyFrom(paragraphs []notesParagraph) []byte {
	text := strings.Builder{}
	runs := []byte{}
	for index := range paragraphs {
		paragraph := &paragraphs[index]
		last := index == len(paragraphs)-1
		if len(paragraph.Runs) == 0 {
			paragraph.Runs = []notesRun{{}}
		}
		for position, run := range paragraph.Runs {
			if position == len(paragraph.Runs)-1 && !last && !strings.HasSuffix(run.Text, "\n") {
				run.Text += "\n"
			}
			if run.Text == "" {
				continue
			}
			text.WriteString(run.Text)
			var owner *notesParagraph
			if position == 0 {
				owner = paragraph
			}
			runs = append(runs, protoField(5, 2, notesRunMessage(run, owner))...)
		}
	}
	value := text.String()
	units := len(utf16.Encode([]rune(value)))
	note := protoField(2, 2, []byte(value))
	note = append(note, protoField(3, 2, protoVarint(2, units))...)
	note = append(note, runs...)
	document := append(protoVarint(2, 0), protoField(3, 2, note)...)
	raw := protoField(2, 2, document)
	var packed bytes.Buffer
	writer := gzip.NewWriter(&packed)
	writer.Write(raw)
	writer.Close()
	return packed.Bytes()
}

func notesPlainParagraphs(text string) []notesParagraph {
	return []notesParagraph{{Style: -1, Runs: []notesRun{{Text: text}}}}
}

func notesHTMLParagraphs(source string) []notesParagraph {
	tokenizer := html.NewTokenizer(strings.NewReader(source))
	paragraphs := []notesParagraph{}
	current := notesParagraph{Style: -1}
	open := false
	bold, italic, underline, strike := 0, 0, 0, 0
	flushParagraph := func() {
		if open {
			paragraphs = append(paragraphs, current)
		}
		current = notesParagraph{Style: -1}
		open = false
	}
	blockTag := func(tag string) bool {
		return tag == "div" || tag == "p" || tag == "h1" || tag == "h2" || tag == "h3" || tag == "li"
	}
	for {
		kind := tokenizer.Next()
		if kind == html.ErrorToken {
			break
		}
		name, _ := tokenizer.TagName()
		tag := string(name)
		switch kind {
		case html.StartTagToken, html.SelfClosingTagToken:
			switch {
			case blockTag(tag):
				flushParagraph()
				open = true
			case tag == "b" || tag == "strong":
				bold++
			case tag == "i" || tag == "em":
				italic++
			case tag == "u":
				underline++
			case tag == "s" || tag == "strike":
				strike++
			case tag == "br" && open && len(current.Runs) > 0:
				current.Runs = append(current.Runs, notesRun{Text: " "})
			}
		case html.EndTagToken:
			switch {
			case blockTag(tag):
				flushParagraph()
			case tag == "b" || tag == "strong":
				bold--
			case tag == "i" || tag == "em":
				italic--
			case tag == "u":
				underline--
			case tag == "s" || tag == "strike":
				strike--
			}
		case html.TextToken:
			text := string(tokenizer.Text())
			if text == "" {
				continue
			}
			open = true
			weight := 0
			if bold > 0 {
				weight |= 1
			}
			if italic > 0 {
				weight |= 2
			}
			current.Runs = append(current.Runs, notesRun{Text: text, Weight: weight, Underline: underline > 0, Strike: strike > 0})
		}
	}
	flushParagraph()
	if len(paragraphs) == 0 {
		paragraphs = append(paragraphs, notesParagraph{Style: -1})
	}
	return paragraphs
}

type notesRow struct {
	observation map[string]any
	collection  string
	key         string
	deleted     bool
}

func refKey(value any) string {
	reference, _ := value.(map[string]any)
	inner, _ := reference["$ref"].(map[string]any)
	key, _ := inner["native_key"].(string)
	return key
}

func notesDate(value any) any {
	text, _ := value.(string)
	if text == "" {
		return nil
	}
	parsed, err := time.Parse(time.RFC3339Nano, text)
	if err != nil {
		return nil
	}
	return float64(parsed.UnixNano())/1e9 - 978307200
}

func flag(value bool) int {
	if value {
		return 1
	}
	return 0
}

const notesSchema = "CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('lab-notes');" +
	"CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount'),(4,'ICAttachment');" +
	"CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);" +
	"CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT,ZALTTEXT TEXT,ZSMARTFOLDERQUERYJSON TEXT,ZCREATIONDATE1 REAL,ZMODIFICATIONDATE1 REAL);"

func (w *World) startSessionNotes() {
	w.sessionNotes = &sessionNotes{folderPK: map[string]int{}, notePK: map[string]int{}, accountPK: map[string]int{}, revisions: map[string]int{}, versions: map[string]string{}, stop: make(chan struct{})}
	go func(stop chan struct{}, ctx context.Context) {
		ticker := time.NewTicker(25 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-stop:
				return
			case <-ctx.Done():
				return
			case <-ticker.C:
				w.syncSessionNotes()
			}
		}
	}(w.sessionNotes.stop, w.ctx)
}

func (w *World) stopSessionNotes() {
	if w.sessionNotes != nil && w.sessionNotes.stop != nil {
		close(w.sessionNotes.stop)
		w.sessionNotes.stop = nil
	}
}

func (w *World) syncSessionNotes() error {
	state := w.sessionNotes
	root := w.sessionRoots["notes"]
	if state == nil || root == "" {
		return nil
	}
	observations, err := w.Inspect()
	if err != nil {
		return err
	}
	state.mu.Lock()
	defer state.mu.Unlock()
	rows := []notesRow{}
	for _, observation := range observations {
		if observation.Collection != "notes.folder" && observation.Collection != "notes.note" {
			continue
		}
		var data map[string]any
		json.Unmarshal(observation.Data, &data)
		if data == nil {
			data = map[string]any{}
		}
		rows = append(rows, notesRow{observation: data, collection: observation.Collection, key: observation.NativeKey, deleted: observation.Kind == "absent"})
	}
	return w.writeSessionNotes(root, rows)
}

func (w *World) writeSessionNotes(root string, source []notesRow) error {
	state := w.sessionNotes
	folders := map[string]notesRow{}
	notes := map[string]notesRow{}
	folderKeys, noteKeys := []string{}, []string{}
	for _, row := range source {
		if row.collection == "notes.folder" {
			folders[row.key] = row
			folderKeys = append(folderKeys, row.key)
		} else {
			notes[row.key] = row
			noteKeys = append(noteKeys, row.key)
		}
	}
	sort.Strings(folderKeys)
	sort.Strings(noteKeys)
	accountNames := map[string]string{}
	accountKeys := []string{}
	for _, key := range folderKeys {
		data := folders[key].observation
		account, _ := data["account"].(string)
		if _, seen := accountNames[account]; !seen {
			accountKeys = append(accountKeys, account)
			accountNames[account] = account
		}
		if name, _ := data["account_name"].(string); name != "" {
			accountNames[account] = name
		}
	}
	sort.Strings(accountKeys)
	next := 0
	for _, table := range []map[string]int{state.accountPK, state.folderPK, state.notePK} {
		for _, pk := range table {
			next = max(next, pk)
		}
	}
	assign := func(table map[string]int, key string) int {
		if pk, ok := table[key]; ok {
			return pk
		}
		next++
		table[key] = next
		return next
	}
	version := func(key string, parts ...any) int {
		encoded, _ := json.Marshal(parts)
		signature := string(encoded)
		if previous, ok := state.versions[key]; !ok {
			state.versions[key] = signature
			state.revisions[key] = 1
		} else if previous != signature {
			state.versions[key] = signature
			state.revisions[key]++
		}
		return state.revisions[key]
	}
	rows := [][]any{}
	for _, account := range accountKeys {
		rows = append(rows, []any{assign(state.accountPK, account), 3, account, nil, nil, accountNames[account], nil, nil, nil, nil, 0, 0, 0, 0, 1, nil, nil, nil, nil, nil, nil, nil, nil})
	}
	for _, key := range folderKeys {
		row := folders[key]
		data := row.observation
		account, _ := data["account"].(string)
		title, _ := data["title"].(string)
		recent, _ := data["recently_deleted"].(bool)
		var parent any
		if parentKey := refKey(data["parent_id"]); parentKey != "" {
			if _, ok := folders[parentKey]; ok {
				parent = assign(state.folderPK, parentKey)
			}
		}
		var smart any
		if smartFlag, _ := data["smart"].(bool); smartFlag {
			smart = "{}"
		}
		revision := version("folder:"+key, title, account, recent, parent, smart, row.deleted)
		rows = append(rows, []any{assign(state.folderPK, key), 1, key, nil, title, nil, state.accountPK[account], parent, nil, nil, flag(row.deleted), flag(recent), 0, 0, revision, nil, nil, nil, nil, nil, smart, nil, nil})
	}
	bodies := [][]any{}
	for _, key := range noteKeys {
		row := notes[key]
		data := row.observation
		folderKey := refKey(data["folder_id"])
		if _, ok := folders[folderKey]; !ok {
			folderKey = ""
			if len(folderKeys) > 0 {
				folderKey = folderKeys[0]
			}
		}
		if folderKey == "" {
			continue
		}
		title, _ := data["title"].(string)
		text, _ := data["plain_text"].(string)
		markup, _ := data["html"].(string)
		locked, _ := data["locked"].(bool)
		pinned, _ := data["pinned"].(bool)
		status, _ := data["content_status"].(string)
		var paragraphs []notesParagraph
		if !locked && status != "offloaded" && (text != "" || markup != "") {
			if markup != "" {
				paragraphs = notesHTMLParagraphs(markup)
			} else {
				paragraphs = notesPlainParagraphs(text)
			}
			blocks, _ := data["blocks"].([]any)
			for index, value := range blocks {
				block, _ := value.(map[string]any)
				if index < len(paragraphs) && block["list_style"] == "checklist" {
					checked, _ := block["checked"].(bool)
					identifier := make([]byte, 16)
					copy(identifier, []byte(key))
					paragraphs[index].Style, paragraphs[index].Checked, paragraphs[index].ID = 103, checked, identifier
				}
			}
		}
		pk := assign(state.notePK, key)
		tags := []string{}
		if values, ok := data["tags"].([]any); ok && paragraphs != nil {
			for _, value := range values {
				if tag, ok := value.(string); ok {
					tags = append(tags, tag)
				}
			}
		}
		for index, tag := range tags {
			identifier := key + "-tag-" + string(rune('a'+index))
			last := &paragraphs[len(paragraphs)-1]
			last.Runs = append(last.Runs, notesRun{Text: "￼", Attach: identifier})
			rows = append(rows, []any{assign(state.notePK, "attachment:"+identifier), 4, identifier, nil, nil, nil, nil, nil, nil, nil, 0, 0, 0, 0, 1, pk, nil, nil, hashtagUTI, tag, nil, nil, nil})
		}
		var body, bodyPK any
		if paragraphs != nil {
			body = base64.StdEncoding.EncodeToString(notesBodyFrom(paragraphs))
			bodyPK = pk
		}
		created, modified := notesDate(data["created_at"]), notesDate(data["modified_at"])
		revision := version("note:"+key, title, body, locked, pinned, folderKey, row.deleted, created, modified, tags)
		bodies = append(bodies, []any{pk, body, revision})
		rows = append(rows, []any{pk, 2, key, title, nil, nil, nil, nil, state.folderPK[folderKey], bodyPK, flag(row.deleted), 0, flag(locked), flag(pinned), revision, nil, nil, nil, nil, nil, nil, created, modified})
	}
	encodedRows, err := json.Marshal(rows)
	if err != nil {
		return err
	}
	encodedBodies, err := json.Marshal(bodies)
	if err != nil {
		return err
	}
	signature := string(encodedRows) + string(encodedBodies)
	if signature == state.signature {
		return nil
	}
	if err = os.MkdirAll(root, 0700); err != nil {
		return err
	}
	temporary := filepath.Join(root, ".tmp-NoteStore.sqlite")
	os.Remove(temporary)
	script := "import base64,json,sqlite3,sys\nc=sqlite3.connect(sys.argv[1])\nc.executescript(sys.argv[2])\nc.executemany('INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZTITLE2,ZNAME,ZACCOUNT8,ZPARENT,ZFOLDER,ZNOTEDATA,ZMARKEDFORDELETION,ZFOLDERTYPE,ZISPASSWORDPROTECTED,ZISPINNED,Z_OPT,ZNOTE1,ZPARENTATTACHMENT,ZMEDIA,ZTYPEUTI1,ZALTTEXT,ZSMARTFOLDERQUERYJSON,ZCREATIONDATE1,ZMODIFICATIONDATE1) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',json.loads(sys.argv[3]))\nc.executemany('INSERT INTO ZICNOTEDATA VALUES(?,?,?)',[(a,base64.b64decode(b) if b else None,d) for a,b,d in json.loads(sys.argv[4])])\nc.commit()\nc.close()\n"
	if _, err = commandOutput(w.ctx, "python3", "-c", script, temporary, notesSchema, string(encodedRows), string(encodedBodies)); err != nil {
		return err
	}
	if err = os.Rename(temporary, filepath.Join(root, "NoteStore.sqlite")); err != nil {
		return err
	}
	state.signature = signature
	return nil
}
