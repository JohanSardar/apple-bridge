package contracts

import (
	"crypto/rand"
	"encoding/json"
	"math/big"
	"strings"
	"time"
)

type Source string

const (
	Messages  Source = "messages"
	Calendar  Source = "calendar"
	Reminders Source = "reminders"
	Contacts  Source = "contacts"
	Notes     Source = "notes"
	Usage     Source = "usage"
	System    Source = "system"
)

var Sources = []Source{Messages, Calendar, Reminders, Contacts, Notes, Usage, System}
var Collections = map[string]string{
	"messages.chat": "cht", "messages.handle": "hdl", "messages.message": "msg", "messages.reaction": "rxn", "messages.attachment": "att",
	"calendar.calendar": "cal", "calendar.event": "evt", "reminders.list": "lst", "reminders.reminder": "rmd",
	"contacts.contact": "con", "contacts.group": "grp", "notes.folder": "fld", "notes.note": "nte", "usage.device": "dev", "usage.app_day": "usd", "system.source": "src", "system.command": "cmd",
}

type Coverage struct {
	Fields  string   `json:"fields"`
	Missing []string `json:"missing"`
}

type Record struct {
	Collection string          `json:"collection"`
	ID         string          `json:"id"`
	Version    uint64          `json:"version"`
	Source     Source          `json:"source"`
	ObservedAt time.Time       `json:"observed_at"`
	Deleted    bool            `json:"deleted"`
	Coverage   Coverage        `json:"coverage"`
	Data       json.RawMessage `json:"data"`
}

type Cursor struct {
	Epoch                string `json:"epoch"`
	SubscriptionRevision uint64 `json:"subscription_revision"`
	Sequence             uint64 `json:"sequence"`
}

type Change struct {
	Sequence uint64 `json:"sequence"`
	Op       string `json:"op"`
	Record   Record `json:"record"`
}

type ReadQuery struct {
	Collection     string `json:"collection"`
	ID             string `json:"id,omitempty"`
	IncludeDeleted bool   `json:"include_deleted,omitempty"`
	After          string `json:"after,omitempty"`
	Limit          int    `json:"limit,omitempty"`
}

type ReadResult struct {
	Records []Record `json:"records"`
	Cursor  Cursor   `json:"cursor"`
	Next    string   `json:"next,omitempty"`
}

func NewID(prefix string) string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	millis := uint64(time.Now().UnixMilli())
	for i := 5; i >= 0; i-- {
		b[i] = byte(millis)
		millis >>= 8
	}
	n := new(big.Int).SetBytes(b[:])
	radix := big.NewInt(32)
	rem := new(big.Int)
	out := make([]byte, 26)
	for i := 25; i >= 0; i-- {
		n.QuoRem(n, radix, rem)
		out[i] = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"[rem.Int64()]
	}
	if prefix == "" {
		return string(out)
	}
	return prefix + "_" + string(out)
}

func CollectionSource(collection string) Source { return Source(strings.SplitN(collection, ".", 2)[0]) }

func ValidateQuery(q ReadQuery) error {
	if q.Collection != "" {
		if _, ok := Collections[q.Collection]; !ok {
			return ErrInvalid
		}
	}
	if q.Limit < 0 || q.Limit > 10000 {
		return ErrInvalid
	}
	return nil
}
