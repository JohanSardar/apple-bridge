package contracts

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"strings"
	"time"
)

type Command struct {
	ID              string          `json:"command_id,omitempty"`
	Op              string          `json:"op,omitempty"`
	Args            json.RawMessage `json:"args"`
	ExpectedVersion uint64          `json:"expected_version,omitempty"`
	ExpiresAt       time.Time       `json:"expires_at,omitempty"`
	AdmissionEra    string          `json:"admission_era,omitempty"`
	WaitMS          int             `json:"wait_ms,omitempty"`
}
type ReminderArgs struct {
	ID        string  `json:"id,omitempty"`
	ListID    string  `json:"list_id,omitempty"`
	Title     *string `json:"title,omitempty"`
	Notes     *string `json:"notes,omitempty"`
	URL       *string `json:"url,omitempty"`
	Due       *string `json:"due,omitempty"`
	DueAllDay *bool   `json:"due_all_day,omitempty"`
	Start     *string `json:"start,omitempty"`
	Priority  *int    `json:"priority,omitempty"`
}
type CommandQuery struct {
	ID      string `json:"command_id,omitempty"`
	Resolve bool   `json:"resolve,omitempty"`
	Limit   int    `json:"limit,omitempty"`
}
type CommandEvent struct {
	State    string          `json:"state"`
	At       time.Time       `json:"at"`
	Evidence json.RawMessage `json:"evidence,omitempty"`
}
type CommandRecord struct {
	Command      Command         `json:"command"`
	Principal    string          `json:"principal"`
	Source       Source          `json:"source"`
	RequestHash  string          `json:"request_hash"`
	State        string          `json:"state"`
	AdmissionEra string          `json:"admission_era"`
	CreatedAt    time.Time       `json:"created_at"`
	ExpiresAt    time.Time       `json:"expires_at"`
	Evidence     json.RawMessage `json:"evidence,omitempty"`
	Disposition  string          `json:"disposition,omitempty"`
	CommitSeq    uint64          `json:"commit_seq"`
	Cursor       Cursor          `json:"cursor"`
	History      []CommandEvent  `json:"history"`
	Error        *Error          `json:"error,omitempty"`
}
type NativeAction struct {
	ExpectedListKey string       `json:"expected_list_key,omitempty"`
	Op              string       `json:"op"`
	Args            ReminderArgs `json:"args"`
	NativeKey       string       `json:"native_key,omitempty"`
	ListKey         string       `json:"list_key,omitempty"`
}

func DecodeInput(b []byte, out any) error {
	b = bytes.TrimSpace(b)
	if len(b) == 0 || b[0] != '{' {
		return ErrInvalid
	}
	d := json.NewDecoder(bytes.NewReader(b))
	d.DisallowUnknownFields()
	if d.Decode(out) != nil || d.Decode(&struct{}{}) != io.EOF {
		return ErrInvalid
	}
	return nil
}
func ValidateCommand(c Command) (ReminderArgs, error) {
	var a ReminderArgs
	if c.ID == "" || c.WaitMS < 0 || c.WaitMS > 15000 || len(c.ID) > 200 || c.Args == nil {
		return a, ErrInvalid
	}
	if DecodeInput(c.Args, &a) != nil {
		return a, ErrInvalid
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(c.Args, &fields) != nil {
		return a, ErrInvalid
	}
	allowed := map[string]bool{}
	for _, field := range ReminderArgumentFields(c.Op) {
		allowed[field] = true
	}
	for field, value := range fields {
		if !allowed[field] || bytes.Equal(bytes.TrimSpace(value), []byte("null")) {
			return a, ErrInvalid
		}
	}
	patch := a.Title != nil || a.Notes != nil || a.URL != nil || a.Due != nil || a.DueAllDay != nil || a.Start != nil || a.Priority != nil
	if a.Title != nil && strings.TrimSpace(*a.Title) == "" {
		return a, ErrInvalid
	}
	switch c.Op {
	case "reminders.create":
		if a.ID != "" || c.ExpectedVersion != 0 || a.ListID == "" || a.Title == nil {
			return a, ErrInvalid
		}
	case "reminders.create_list":
		if a.ID != "" || a.ListID != "" || c.ExpectedVersion != 0 || a.Title == nil || a.Notes != nil || a.URL != nil || a.Due != nil || a.DueAllDay != nil || a.Start != nil || a.Priority != nil {
			return a, ErrInvalid
		}
	case "reminders.update":
		if a.ID == "" || c.ExpectedVersion == 0 || (!patch && a.ListID == "") {
			return a, ErrInvalid
		}
	case "reminders.complete", "reminders.uncomplete", "reminders.delete":
		if a.ID == "" || c.ExpectedVersion == 0 || a.ListID != "" || patch {
			return a, ErrInvalid
		}
	default:
		return a, ErrUnsupported
	}
	if a.Priority != nil && (*a.Priority < 0 || *a.Priority > 9) {
		return a, ErrInvalid
	}
	return a, nil
}
func CommandHash(c Command) (string, error) {
	var args any
	d := json.NewDecoder(bytes.NewReader(c.Args))
	d.UseNumber()
	if d.Decode(&args) != nil {
		return "", ErrInvalid
	}
	b, err := json.Marshal(struct {
		Op      string
		Args    any
		Version uint64
		Expires time.Time
		Era     string
	}{c.Op, args, c.ExpectedVersion, c.ExpiresAt.UTC(), c.AdmissionEra})
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:]), err
}
func Descriptor(name string) (OperationDescriptor, bool) {
	for _, d := range Operations {
		if d.Name == name {
			return d, true
		}
	}
	return OperationDescriptor{}, false
}
func CommandTTL(op string) (time.Duration, time.Duration) {
	if strings.HasPrefix(op, "messages.") {
		return time.Minute, 5 * time.Minute
	}
	return 5 * time.Minute, time.Hour
}

func NewCommandID(epoch string) string { return "cmd_" + epoch + "_" + NewID("") }
func CommandEpoch(id string) string {
	parts := strings.SplitN(id, "_", 3)
	if len(parts) != 3 || parts[0] != "cmd" || parts[2] == "" {
		return ""
	}
	return parts[1]
}

func ReminderArgumentFields(op string) []string {
	switch op {
	case "reminders.create":
		return []string{"list_id", "title", "notes", "url", "due", "due_all_day", "start", "priority"}
	case "reminders.create_list":
		return []string{"title"}
	case "reminders.update":
		return []string{"id", "list_id", "title", "notes", "url", "due", "due_all_day", "start", "priority"}
	case "reminders.complete", "reminders.uncomplete", "reminders.delete":
		return []string{"id"}
	}
	return nil
}
