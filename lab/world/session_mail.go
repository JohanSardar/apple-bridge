package world

import (
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

type sessionMailStore struct {
	kinds     map[string]string
	mailboxes map[int64]sessionMailbox
	messages  map[int64]sessionMessage
}

type sessionMailbox struct {
	Account string
	Path    string
}

type sessionMessage struct {
	Mailbox   int64
	Subject   string
	Sender    string
	Received  float64
	Read      bool
	MessageID string
}

type sessionDatasetRecord struct {
	Collection string          `json:"collection"`
	NativeKey  string          `json:"native_key"`
	Kind       string          `json:"kind"`
	Data       json.RawMessage `json:"data"`
}

func (s *sessionMailStore) apply(records []sessionDatasetRecord) error {
	if s.kinds == nil {
		s.kinds, s.mailboxes, s.messages = map[string]string{}, map[int64]sessionMailbox{}, map[int64]sessionMessage{}
	}
	for _, record := range records {
		if record.Kind != "upsert" {
			continue
		}
		switch record.Collection {
		case "mail.account":
			var data struct {
				Kind string `json:"kind"`
			}
			if err := json.Unmarshal(record.Data, &data); err != nil {
				return err
			}
			s.kinds[record.NativeKey] = data.Kind
		case "mail.mailbox":
			id, err := strconv.ParseInt(strings.TrimPrefix(record.NativeKey, "mailbox/"), 10, 64)
			if err != nil {
				return err
			}
			var data struct {
				Account string `json:"account_id"`
				Path    string `json:"path"`
			}
			if err = json.Unmarshal(record.Data, &data); err != nil {
				return err
			}
			s.mailboxes[id] = sessionMailbox{Account: data.Account, Path: data.Path}
		case "mail.message":
			id, err := strconv.ParseInt(record.NativeKey, 10, 64)
			if err != nil {
				return err
			}
			var data struct {
				Subject    string   `json:"subject"`
				From       string   `json:"from"`
				ReceivedAt string   `json:"received_at"`
				Read       bool     `json:"read"`
				MessageID  string   `json:"message_id"`
				MailboxIDs []string `json:"mailbox_ids"`
			}
			if err = json.Unmarshal(record.Data, &data); err != nil || len(data.MailboxIDs) != 1 {
				return os.ErrInvalid
			}
			mailbox, err := strconv.ParseInt(strings.TrimPrefix(data.MailboxIDs[0], "mailbox/"), 10, 64)
			if err != nil {
				return err
			}
			received, err := time.Parse(time.RFC3339Nano, data.ReceivedAt)
			if err != nil {
				return err
			}
			s.messages[id] = sessionMessage{Mailbox: mailbox, Subject: data.Subject, Sender: data.From, Received: float64(received.UnixNano()) / 1e9, Read: data.Read, MessageID: data.MessageID}
		}
	}
	return nil
}

func (s *sessionMailStore) rows() (mailboxes [][]any, messages [][]any) {
	for id, mailbox := range s.mailboxes {
		segments := strings.Split(strings.TrimPrefix(mailbox.Path, "/"), "/")
		for i, segment := range segments {
			segments[i] = url.PathEscape(segment)
		}
		address := strings.ToLower(s.kinds[mailbox.Account]) + "://" + mailbox.Account + "/" + strings.Join(segments, "/")
		mailboxes = append(mailboxes, []any{id, address})
	}
	for id, message := range s.messages {
		read := 0
		if message.Read {
			read = 1
		}
		messages = append(messages, []any{id, message.Mailbox, message.Subject, message.Sender, message.Received, read, message.MessageID})
	}
	return mailboxes, messages
}

const sessionMailScript = `import json,sqlite3,sys
rows=json.load(open(sys.argv[2]))
c=sqlite3.connect(sys.argv[1])
c.execute('PRAGMA journal_mode=DELETE')
c.execute('CREATE TABLE mailboxes(url TEXT)')
c.execute('CREATE TABLE subjects(subject TEXT)')
c.execute('CREATE TABLE addresses(address TEXT)')
c.execute('CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT)')
c.executemany('INSERT INTO mailboxes(ROWID,url) VALUES(?,?)',rows['mailboxes'])
subjects={}
addresses={}
def intern(table,column,index,value):
    if value not in index:
        index[value]=len(index)+1
        c.execute('INSERT INTO %s(ROWID,%s) VALUES(?,?)'%(table,column),(index[value],value))
    return index[value]
for rowid,mailbox,subject,sender,received,read,message_id in rows['messages']:
    c.execute('INSERT INTO messages(ROWID,mailbox,subject,sender,date_received,read,message_id) VALUES(?,?,?,?,?,?,?)',(rowid,mailbox,intern('subjects','subject',subjects,subject),intern('addresses','address',addresses,sender),received,read,message_id))
c.commit()
c.close()
`

func (w *World) stageSessionMail(action any) error {
	raw, err := json.Marshal(action)
	if err != nil {
		return err
	}
	var dataset struct {
		Records []sessionDatasetRecord `json:"records"`
	}
	if err = json.Unmarshal(raw, &dataset); err != nil {
		return err
	}
	if err = w.sessionMail.apply(dataset.Records); err != nil {
		return err
	}
	mailboxes, messages := w.sessionMail.rows()
	encoded, err := json.Marshal(map[string]any{"mailboxes": mailboxes, "messages": messages})
	if err != nil {
		return err
	}
	root := w.sessionRoots["mail"]
	rowsFile := filepath.Join(w.Dir, "session-mail-rows.json")
	if err = os.WriteFile(rowsFile, encoded, 0600); err != nil {
		return err
	}
	temporary := filepath.Join(w.Dir, ".tmp-session-mail.db")
	os.Remove(temporary)
	if _, err = commandOutput(w.ctx, "python3", "-c", sessionMailScript, temporary, rowsFile); err != nil {
		return err
	}
	return os.Rename(temporary, filepath.Join(root, "Envelope Index"))
}
