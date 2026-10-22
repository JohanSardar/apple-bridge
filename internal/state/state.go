package state

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	_ "modernc.org/sqlite"
)

type Store struct {
	BlobProtection func(context.Context, contracts.ReferenceSet) error
	BlobDelta      func(context.Context, contracts.ReferenceSet, bool) error
	blobSnapshots  map[string][]contracts.BlobReference

	home            string
	ledger          *sql.DB
	applyBoundary   func()
	db              *sql.DB
	write           sync.Mutex
	mu              sync.Mutex
	changed         chan struct{}
	leases          map[string]snapshotLease
	captureFailures map[contracts.Source]captureFailure
	capacity        map[contracts.Source]bool
	searchCancel    context.CancelFunc
	searchDone      chan struct{}
}
type CommitResult struct {
	CommittedAt time.Time `json:"-"`
	Duplicate   bool      `json:"duplicate,omitempty"`
	After       uint64    `json:"after"`
	Through     uint64    `json:"through"`
	Degraded    bool      `json:"degraded"`
}

func Open(home string) (*Store, error) {
	if err := os.MkdirAll(home, 0700); err != nil {
		return nil, err
	}
	path := filepath.Join(home, "state.db")
	file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, err
	}
	file.Close()
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)&_pragma=busy_timeout(5000)&_pragma=foreign_keys(1)")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(8)
	s := &Store{home: home, db: db, changed: make(chan struct{}), leases: map[string]snapshotLease{}, capacity: map[contracts.Source]bool{}}
	if err = migrate(db, home); err != nil {
		db.Close()
		return nil, err
	}
	_, err = db.Exec(`CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS records(collection TEXT NOT NULL,id TEXT NOT NULL,version INTEGER NOT NULL,source TEXT NOT NULL,deleted INTEGER NOT NULL,payload BLOB NOT NULL,seq INTEGER NOT NULL,observed_at TEXT NOT NULL,PRIMARY KEY(collection,id));
 CREATE TABLE IF NOT EXISTS aliases(source TEXT NOT NULL,generation TEXT NOT NULL,collection TEXT NOT NULL,native_key TEXT NOT NULL,scoped_alias TEXT NOT NULL,record_id TEXT NOT NULL,PRIMARY KEY(source,generation,collection,native_key));
 CREATE INDEX IF NOT EXISTS aliases_scoped ON aliases(source,collection,scoped_alias);
 CREATE INDEX IF NOT EXISTS aliases_record ON aliases(collection,record_id,source,generation,native_key);
 CREATE TABLE IF NOT EXISTS journal(seq INTEGER PRIMARY KEY,epoch TEXT NOT NULL,source TEXT NOT NULL,payload BLOB NOT NULL,committed_at INTEGER NOT NULL DEFAULT (unixepoch()));
 CREATE TABLE IF NOT EXISTS agent_activity(id TEXT PRIMARY KEY,principal TEXT NOT NULL,operation TEXT NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL,at INTEGER NOT NULL,count INTEGER NOT NULL,argument_bytes INTEGER NOT NULL,argument_hash TEXT NOT NULL,sequence INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS agent_activity_at ON agent_activity(at);
 CREATE TABLE IF NOT EXISTS checkpoints(source TEXT PRIMARY KEY,generation TEXT NOT NULL,checkpoint TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS blob_reference_sets(owner TEXT PRIMARY KEY,payload BLOB NOT NULL);
 CREATE TABLE IF NOT EXISTS applied_batches(after_seq INTEGER PRIMARY KEY,through_seq INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS staging_records(snapshot TEXT NOT NULL,collection TEXT NOT NULL,id TEXT NOT NULL,payload BLOB NOT NULL,PRIMARY KEY(snapshot,collection,id));
 DELETE FROM staging_records;`)
	if err != nil {
		db.Close()
		return nil, err
	}
	c := contracts.Cursor{Epoch: contracts.NewID(""), SubscriptionRevision: 1}
	raw, _ := json.Marshal(c)
	_, err = db.Exec("INSERT OR IGNORE INTO meta(key,value) VALUES('cursor',? ),('owner_fence','0'),('schema_version','2'),('retention_floor','0'),('admission_era',?),('epoch',?)", string(raw), contracts.NewID(""), c.Epoch)
	if err != nil {
		db.Close()
		return nil, err
	}
	if err = s.initReadModels(context.Background()); err != nil {
		db.Close()
		return nil, err
	}
	searchPending, err := s.initSearch()
	if err != nil {
		db.Close()
		return nil, err
	}
	if err = s.initBlobReferences(context.Background()); err != nil {
		db.Close()
		return nil, err
	}
	initializeApplyBoundary(s)
	if searchPending {
		s.startSearchRebuild()
	}
	return s, nil
}
func (s *Store) Close() error {
	if s.searchCancel != nil {
		s.searchCancel()
		<-s.searchDone
	}
	if s.ledger != nil {
		return errors.Join(s.ledger.Close(), s.db.Close())
	}
	return s.db.Close()
}
func (s *Store) signal() {
	s.mu.Lock()
	close(s.changed)
	s.changed = make(chan struct{})
	s.mu.Unlock()
}
func (s *Store) watch() <-chan struct{} { s.mu.Lock(); defer s.mu.Unlock(); return s.changed }

type querier interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
}

func cursorAt(ctx context.Context, q querier) (contracts.Cursor, error) {
	var c contracts.Cursor
	var b string
	if err := q.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='cursor'").Scan(&b); err != nil {
		return c, err
	}
	err := json.Unmarshal([]byte(b), &c)
	return c, err
}
func setCursor(ctx context.Context, tx *sql.Tx, c contracts.Cursor) error {
	b, err := json.Marshal(c)
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='cursor'", string(b))
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='epoch'", c.Epoch)
	return err
}
func (s *Store) Head() contracts.Cursor { c, _ := cursorAt(context.Background(), s.db); return c }
func (s *Store) NextFence(ctx context.Context) (uint64, error) {
	s.write.Lock()
	defer s.write.Unlock()
	var n uint64
	err := s.db.QueryRowContext(ctx, "UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='owner_fence' RETURNING value").Scan(&n)
	return n, err
}
func (s *Store) Checkpoints(ctx context.Context) (map[contracts.Source]contracts.Checkpoint, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT source,generation,checkpoint FROM checkpoints")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := map[contracts.Source]contracts.Checkpoint{}
	for rows.Next() {
		var source contracts.Source
		var c contracts.Checkpoint
		if err = rows.Scan(&source, &c.Generation, &c.Checkpoint); err != nil {
			return nil, err
		}
		out[source] = c
	}
	return out, rows.Err()
}

func recordAt(ctx context.Context, tx *sql.Tx, collection, id string) (contracts.Record, error) {
	var r contracts.Record
	var b []byte
	err := tx.QueryRowContext(ctx, "SELECT payload FROM records WHERE collection=? AND id=?", collection, id).Scan(&b)
	if err != nil {
		return r, err
	}
	err = json.Unmarshal(b, &r)
	return r, err
}
func writeRecord(ctx context.Context, tx *sql.Tx, r contracts.Record, seq uint64) error {
	b, err := json.Marshal(r)
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO records(collection,id,version,source,deleted,payload,seq,observed_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(collection,id) DO UPDATE SET version=excluded.version,source=excluded.source,deleted=excluded.deleted,payload=excluded.payload,seq=excluded.seq,observed_at=excluded.observed_at`, r.Collection, r.ID, r.Version, r.Source, r.Deleted, b, seq, r.ObservedAt.Format(time.RFC3339Nano))
	if err != nil {
		return err
	}
	if err = writeReadModel(ctx, tx, r, seq); err != nil {
		return err
	}
	return writeBlobReferences(ctx, tx, "record:"+r.ID, r)
}
func appendChange(ctx context.Context, tx *sql.Tx, c contracts.Cursor, r contracts.Record, op string) error {
	change := contracts.Change{Sequence: c.Sequence, Record: r, Op: op}
	b, err := json.Marshal(change)
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO journal(seq,epoch,source,payload,committed_at) VALUES(?,?,?,?,unixepoch())", c.Sequence, c.Epoch, r.Source, b)
	return err
}

func resolveID(ctx context.Context, tx *sql.Tx, c contracts.Capture, o contracts.Observation, create bool) (string, error) {
	var id string
	err := tx.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE source=? AND generation=? AND collection=? AND native_key=?", c.Source, c.Generation, o.Collection, o.NativeKey).Scan(&id)
	if err == nil {
		return id, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return "", err
	}
	if o.ScopedAlias != "" {
		err = tx.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE source=? AND collection=? AND scoped_alias=? LIMIT 1", c.Source, o.Collection, o.ScopedAlias).Scan(&id)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return "", err
		}
	}
	if id == "" {
		if !create {
			return "", sql.ErrNoRows
		}
		id = contracts.NewID(contracts.Collections[o.Collection])
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO aliases(source,generation,collection,native_key,scoped_alias,record_id) VALUES(?,?,?,?,?,?)", c.Source, c.Generation, o.Collection, o.NativeKey, o.ScopedAlias, id)
	return id, err
}
func resolveReferences(ctx context.Context, tx *sql.Tx, c contracts.Capture, value any) (any, error) {
	switch v := value.(type) {
	case map[string]any:
		if reference, ok := v["$ref"].(map[string]any); ok {
			collection, _ := reference["collection"].(string)
			native, _ := reference["native_key"].(string)
			return resolveID(ctx, tx, c, contracts.Observation{Collection: collection, NativeKey: native}, false)
		}
		for k, item := range v {
			resolved, err := resolveReferences(ctx, tx, c, item)
			if err != nil {
				return nil, err
			}
			v[k] = resolved
		}
	case []any:
		for i, item := range v {
			resolved, err := resolveReferences(ctx, tx, c, item)
			if err != nil {
				return nil, err
			}
			v[i] = resolved
		}
	}
	return value, nil
}

func (s *Store) CommitCapture(ctx context.Context, c contracts.Capture) (CommitResult, error) {
	var result CommitResult
	if c.Generation == "" || c.Checkpoint == "" || !slices.Contains(contracts.Sources, c.Source) {
		return result, contracts.ErrInvalid
	}
	for _, o := range c.Observations {
		if o.Kind == "evict" && (c.Source != contracts.Calendar || o.Collection != "calendar.event") {
			return result, contracts.ErrInvalid
		}
		if _, ok := contracts.Collections[o.Collection]; !ok || (contracts.CollectionSource(o.Collection) != c.Source && o.Collection != "system.source") || o.NativeKey == "" || o.ObservedAt.IsZero() || (o.Kind != "upsert" && o.Kind != "absent" && o.Kind != "alias" && o.Kind != "evict") {
			return result, contracts.ErrInvalid
		}
		if o.Kind == "upsert" {
			var value map[string]any
			if json.Unmarshal(o.Data, &value) != nil || value == nil {
				return result, contracts.ErrInvalid
			}
		}
	}
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	cursor, err := cursorAt(ctx, tx)
	if err != nil {
		return result, err
	}
	result.After = cursor.Sequence
	result.Through = cursor.Sequence
	var gen, cp string
	err = tx.QueryRowContext(ctx, "SELECT generation,checkpoint FROM checkpoints WHERE source=?", c.Source).Scan(&gen, &cp)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return result, err
	}
	if gen == c.Generation && cp == c.Checkpoint && (c.Source != contracts.Contacts || len(c.Observations) == 0 || c.Predecessor != cp) {
		result.Duplicate = true
		return result, nil
	}
	expected := ""
	if gen == c.Generation {
		expected = cp
	}
	if c.Predecessor != expected {
		return result, contracts.ErrReset
	}
	for _, o := range c.Observations {
		if o.Kind != "absent" && o.Kind != "evict" {
			if _, err = resolveID(ctx, tx, c, o, true); err != nil {
				return result, err
			}
		}
	}
	var contactHandles map[string]string
	if c.Source == contracts.Messages && slices.ContainsFunc(c.Observations, func(o contracts.Observation) bool { return o.Kind == "upsert" && o.Collection == "messages.handle" }) {
		contactHandles, err = contactIndex(ctx, tx)
		if err != nil {
			return result, err
		}
	}
	for _, o := range c.Observations {
		if o.Kind == "alias" {
			continue
		}
		if (o.Kind == "absent" || o.Kind == "evict") && (c.ScanID == "" || !slices.Contains(c.Scope, o.Collection) || c.CompletionEvidence != "complete" || c.ScanStartedAt.IsZero() || !o.Revalidated) {
			result.Degraded = true
			continue
		}
		id, err := resolveID(ctx, tx, c, o, o.Kind == "upsert")
		if errors.Is(err, sql.ErrNoRows) {
			continue
		}
		if err != nil {
			return result, err
		}
		old, err := recordAt(ctx, tx, o.Collection, id)
		exists := err == nil
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return result, err
		}
		latest := old.ObservedAt
		if exists {
			var observed string
			if err = tx.QueryRowContext(ctx, "SELECT observed_at FROM records WHERE collection=? AND id=?", o.Collection, id).Scan(&observed); err != nil {
				return result, err
			}
			if latest, err = time.Parse(time.RFC3339Nano, observed); err != nil {
				return result, err
			}
		}
		if exists && o.ObservedAt.Before(latest) {
			continue
		}
		var data json.RawMessage
		if o.Kind == "absent" || o.Kind == "evict" {
			if !exists {
				continue
			}
			if latest.After(c.ScanStartedAt) {
				continue
			}
			data = old.Data
		} else {
			var value any
			decoder := json.NewDecoder(bytes.NewReader(o.Data))
			decoder.UseNumber()
			if err = decoder.Decode(&value); err != nil {
				return result, err
			}
			value, err = resolveReferences(ctx, tx, c, value)
			if err != nil {
				return result, err
			}
			data, err = json.Marshal(value)
			if err != nil {
				return result, err
			}
		}
		if o.Kind == "upsert" && o.Collection == "messages.handle" {
			data, err = linkedHandleData(data, contactHandles)
			if err != nil {
				return result, err
			}
		}
		deleted := o.Kind == "absent" || o.Kind == "evict"
		if exists && old.Deleted == deleted && string(old.Data) == string(data) {
			if _, err = tx.ExecContext(ctx, "UPDATE records SET observed_at=? WHERE collection=? AND id=?", o.ObservedAt.Format(time.RFC3339Nano), o.Collection, id); err != nil {
				return result, err
			}
			continue
		}
		record := contracts.Record{Collection: o.Collection, ID: id, Version: old.Version + 1, Source: c.Source, ObservedAt: o.ObservedAt, Deleted: deleted, Data: data, Coverage: contracts.Coverage{Fields: "complete", Missing: []string{}}}
		if o.Kind == "evict" {
			record.TombstoneReason = "window_evicted"
		}
		encoded, err := json.Marshal(record)
		if err != nil {
			return result, err
		}
		if len(encoded) > contracts.MaxFrame-2048 {
			return result, contracts.ErrCapacity
		}
		cursor.Sequence++
		if err = writeRecord(ctx, tx, record, cursor.Sequence); err != nil {
			return result, err
		}
		op := "upsert"
		if deleted {
			op = "delete"
		}
		if o.Kind == "evict" {
			op = "evict"
		}
		if err = appendChange(ctx, tx, cursor, record, op); err != nil {
			return result, err
		}
	}
	if c.Source == contracts.Contacts {
		if err = refreshContactLinks(ctx, tx, &cursor); err != nil {
			return result, err
		}
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO checkpoints(source,generation,checkpoint) VALUES(?,?,?) ON CONFLICT(source) DO UPDATE SET generation=excluded.generation,checkpoint=excluded.checkpoint`, c.Source, c.Generation, c.Checkpoint)
	if err != nil {
		return result, err
	}
	if err = setCursor(ctx, tx, cursor); err != nil {
		return result, err
	}
	if result.CommittedAt, err = s.commitProtectedAt(ctx, tx); err != nil {
		return result, err
	}
	result.Through = cursor.Sequence
	s.signal()
	return result, nil
}

func (s *Store) Read(ctx context.Context, v trust.Visibility, q contracts.ReadQuery) (contracts.ReadResult, error) {
	result := contracts.ReadResult{Records: []contracts.Record{}}
	if err := contracts.ValidateQuery(q); err != nil {
		return result, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	result.Cursor, err = cursorAt(ctx, tx)
	if err != nil {
		return result, err
	}
	sources := []string{}
	afterCollection, afterID, _ := strings.Cut(q.After, "/")
	args := []any{q.Collection, q.Collection, q.ID, q.ID, q.IncludeDeleted, afterCollection, afterID}
	for source, allowed := range v {
		if allowed {
			sources = append(sources, "?")
			args = append(args, string(source))
		}
	}
	if len(sources) == 0 {
		sources = append(sources, "?")
		args = append(args, "")
	}
	query := "SELECT payload FROM records WHERE (?='' OR collection=?) AND (?='' OR id=?) AND (deleted=0 OR ?) AND (collection,id)>(?,?) AND source IN (" + strings.Join(sources, ",") + ")"
	if q.Text != "" {
		if err = requireSearchReady(ctx, tx); err != nil {
			return result, err
		}
		query += " AND (collection,id) IN (SELECT collection,id FROM records_fts WHERE records_fts MATCH ?)"
		args = append(args, searchTerms(q.Text))
	}
	query += " ORDER BY collection,id"
	rows, err := tx.QueryContext(ctx, query, args...)
	if err != nil {
		return result, err
	}
	defer rows.Close()
	limit := q.Limit
	if limit == 0 {
		limit = 1000
	}
	for rows.Next() {
		var b []byte
		if err = rows.Scan(&b); err != nil {
			return result, err
		}
		var r contracts.Record
		if err = json.Unmarshal(b, &r); err != nil {
			return result, err
		}
		key := r.Collection + "/" + r.ID
		if !v.Allows(r) || key <= q.After {
			continue
		}
		if len(result.Records) == limit {
			result.Next = result.Records[len(result.Records)-1].Collection + "/" + result.Records[len(result.Records)-1].ID
			break
		}
		r, err = projectContactLink(r, v)
		if err != nil {
			return result, err
		}
		result.Records = append(result.Records, r)
	}
	if err = rows.Err(); err != nil {
		return result, err
	}
	rows.Close()
	if q.IncludeAliases {
		for _, record := range result.Records {
			aliases, err := tx.QueryContext(ctx, "SELECT a.source,a.generation,a.collection,a.native_key,a.record_id FROM aliases a JOIN checkpoints c ON a.source=c.source AND a.generation=c.generation WHERE a.collection=? AND a.record_id=? ORDER BY a.source,a.generation,a.native_key", record.Collection, record.ID)
			if err != nil {
				return result, err
			}
			for aliases.Next() {
				var alias contracts.Alias
				if err = aliases.Scan(&alias.Source, &alias.Generation, &alias.Collection, &alias.NativeKey, &alias.RecordID); err != nil {
					aliases.Close()
					return result, err
				}
				result.Aliases = append(result.Aliases, alias)
			}
			err = aliases.Err()
			aliases.Close()
			if err != nil {
				return result, err
			}
		}
	}
	if q.ID != "" && len(result.Records) == 0 {
		return result, contracts.ErrNotFound
	}
	return result, nil
}

type FrozenSnapshot struct {
	ID        string
	Cursor    contracts.Cursor
	Count     int
	Digest    string
	ExpiresAt time.Time
	Records   []contracts.Record
	store     *Store
}

func (f *FrozenSnapshot) Close() error {
	f.store.write.Lock()
	defer f.store.write.Unlock()
	delete(f.store.leases, f.ID)
	delete(f.store.blobSnapshots, f.ID)
	if f.store.BlobDelta != nil {
		return f.store.BlobDelta(context.Background(), contracts.ReferenceSet{"snapshot:" + f.ID: nil}, false)
	}
	return nil
}
func Digest(records []contracts.Record) string {
	ordered := slices.Clone(records)
	slices.SortFunc(ordered, func(a, b contracts.Record) int { return strings.Compare(a.Collection+"/"+a.ID, b.Collection+"/"+b.ID) })
	h := sha256.New()
	for _, r := range ordered {
		fmt.Fprintf(h, "%s\x00%s\x00%d\n", r.Collection, r.ID, r.Version)
	}
	return hex.EncodeToString(h.Sum(nil))
}
func (s *Store) FreezeSnapshot(ctx context.Context, v trust.Visibility, revision uint64) (*FrozenSnapshot, error) {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return nil, err
	}
	f := &FrozenSnapshot{store: s, ID: contracts.NewID("snap"), ExpiresAt: time.Now().Add(10 * time.Minute), Records: []contracts.Record{}}
	f.Cursor, err = cursorAt(ctx, tx)
	if err != nil {
		tx.Rollback()
		return nil, err
	}
	f.Cursor.SubscriptionRevision = revision
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE deleted=0 ORDER BY collection,id")
	if err != nil {
		tx.Rollback()
		return nil, err
	}
	for rows.Next() {
		var b []byte
		if err = rows.Scan(&b); err != nil {
			break
		}
		var r contracts.Record
		if err = json.Unmarshal(b, &r); err != nil {
			break
		}
		if v.Allows(r) {
			r, err = projectContactLink(r, v)
			if err != nil {
				break
			}
			f.Records = append(f.Records, r)
		}
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		tx.Rollback()
		return nil, err
	}
	if err = tx.Rollback(); err != nil {
		return nil, err
	}
	f.Count = len(f.Records)
	f.Digest = Digest(f.Records)
	if s.blobSnapshots == nil {
		s.blobSnapshots = map[string][]contracts.BlobReference{}
	}
	for _, r := range f.Records {
		s.blobSnapshots[f.ID] = append(s.blobSnapshots[f.ID], contracts.BlobReferences(r.Data)...)
	}
	if s.BlobDelta != nil {
		err = s.BlobDelta(ctx, contracts.ReferenceSet{"snapshot:" + f.ID: s.blobSnapshots[f.ID]}, false)
	}
	if err != nil {
		delete(s.blobSnapshots, f.ID)
		return nil, err
	}
	s.leases[f.ID] = snapshotLease{Sequence: f.Cursor.Sequence, ExpiresAt: f.ExpiresAt}
	return f, nil
}

type SnapshotInstall struct {
	store     *Store
	ID        string
	Cursor    contracts.Cursor
	Count     int
	Digest    string
	ExpiresAt time.Time
	ordinal   int
}

func (s *Store) BeginInstall(ctx context.Context, begin contracts.FeedFrame, epoch string, revision uint64) (*SnapshotInstall, error) {
	if begin.ID == "" || begin.Count < 0 || begin.Digest == "" || time.Now().After(begin.ExpiresAt) {
		return nil, contracts.ErrInvalid
	}
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, "DELETE FROM staging_records"); err != nil {
		return nil, err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return nil, err
	}
	return &SnapshotInstall{store: s, ID: begin.ID, Cursor: contracts.Cursor{Epoch: epoch, SubscriptionRevision: revision, Sequence: begin.S}, Count: begin.Count, Digest: begin.Digest, ExpiresAt: begin.ExpiresAt}, nil
}
func (i *SnapshotInstall) Page(ctx context.Context, page contracts.FeedFrame) error {
	if page.ID != i.ID || page.Ordinal != i.ordinal || time.Now().After(i.ExpiresAt) {
		return contracts.ErrCorrupt
	}
	i.store.write.Lock()
	defer i.store.write.Unlock()
	tx, err := i.store.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	for _, r := range page.Records {
		if r.ID == "" || r.Version == 0 || r.Deleted {
			return contracts.ErrCorrupt
		}
		if _, ok := contracts.Collections[r.Collection]; !ok {
			return contracts.ErrCorrupt
		}
		b, err := json.Marshal(r)
		if err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO staging_records(snapshot,collection,id,payload) VALUES(?,?,?,?)", i.ID, r.Collection, r.ID, b); err != nil {
			return err
		}
		if err = writeBlobReferences(ctx, tx, "staging:"+i.ID+":"+r.ID, r); err != nil {
			return err
		}
	}
	if err = i.store.commitProtected(ctx, tx); err != nil {
		return err
	}
	i.ordinal++
	i.ExpiresAt = time.Now().Add(10 * time.Minute)
	return nil
}
func (i *SnapshotInstall) InstallSnapshot(ctx context.Context) (contracts.Cursor, error) {
	s := i.store
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return contracts.Cursor{}, err
	}
	defer tx.Rollback()
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM staging_records WHERE snapshot=? ORDER BY collection,id", i.ID)
	if err != nil {
		return contracts.Cursor{}, err
	}
	records := []contracts.Record{}
	for rows.Next() {
		var b []byte
		if err = rows.Scan(&b); err != nil {
			break
		}
		var r contracts.Record
		if err = json.Unmarshal(b, &r); err != nil {
			break
		}
		records = append(records, r)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return contracts.Cursor{}, err
	}
	if time.Now().After(i.ExpiresAt) || len(records) != i.Count || Digest(records) != i.Digest {
		return contracts.Cursor{}, contracts.ErrCorrupt
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM msg_by_chat; DELETE FROM reaction_by_message; DELETE FROM attachment_by_message; DELETE FROM activity_by_chat; DELETE FROM chat_summary; DELETE FROM records; DELETE FROM journal; DELETE FROM applied_batches;"); err != nil {
		return contracts.Cursor{}, err
	}
	for _, r := range records {
		if err = writeRecord(ctx, tx, r, i.Cursor.Sequence); err != nil {
			return contracts.Cursor{}, err
		}
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('replica_base',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", fmt.Sprint(i.Cursor.Sequence)); err != nil {
		return contracts.Cursor{}, err
	}
	if err = setCursor(ctx, tx, i.Cursor); err != nil {
		return contracts.Cursor{}, err
	}
	if _, err = tx.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='retention_floor'", fmt.Sprint(i.Cursor.Sequence)); err != nil {
		return contracts.Cursor{}, err
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM staging_records WHERE snapshot=?", i.ID); err != nil {
		return contracts.Cursor{}, err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return contracts.Cursor{}, err
	}
	s.signal()
	return i.Cursor, nil
}

func (s *Store) ApplyBatch(ctx context.Context, b contracts.Batch) (contracts.Cursor, error) {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return contracts.Cursor{}, err
	}
	defer tx.Rollback()
	c, err := cursorAt(ctx, tx)
	if err != nil {
		return c, err
	}
	if b.Epoch != c.Epoch || b.SubscriptionRevision != c.SubscriptionRevision {
		return c, contracts.ErrReset
	}
	if b.Through <= c.Sequence {
		return c, nil
	}
	if b.After != c.Sequence || b.Through < b.After {
		return c, contracts.ErrConflict
	}
	previous := b.After
	for _, change := range b.Changes {
		if change.Sequence <= previous || change.Sequence > b.Through || change.Record.Version == 0 || change.Record.ID == "" || (change.Op != "upsert" && change.Op != "delete" && change.Op != "evict") {
			return c, contracts.ErrCorrupt
		}
		previous = change.Sequence
		if err = writeRecord(ctx, tx, change.Record, change.Sequence); err != nil {
			return c, err
		}
		if err = appendChange(ctx, tx, contracts.Cursor{Epoch: c.Epoch, Sequence: change.Sequence}, change.Record, change.Op); err != nil {
			return c, err
		}
	}
	if s.applyBoundary != nil && len(b.Changes) > 0 {
		s.applyBoundary()
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO applied_batches(after_seq,through_seq) VALUES(?,?)", b.After, b.Through); err != nil {
		return c, err
	}
	c.Sequence = b.Through
	if err = setCursor(ctx, tx, c); err != nil {
		return c, err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return c, err
	}
	s.signal()
	return c, nil
}

type ChangeIterator struct {
	store      *Store
	visibility trust.Visibility
	cursor     contracts.Cursor
}

func (s *Store) Changes(ctx context.Context, v trust.Visibility, after contracts.Cursor) (*ChangeIterator, error) {
	head, err := cursorAt(ctx, s.db)
	if err != nil {
		return nil, err
	}
	if after.Epoch != head.Epoch || after.SubscriptionRevision != head.SubscriptionRevision || after.Sequence > head.Sequence || after.Sequence < s.RetentionFloor() {
		return nil, contracts.ErrReset
	}
	return &ChangeIterator{store: s, visibility: v, cursor: after}, nil
}
func (i *ChangeIterator) Next(ctx context.Context) (contracts.Batch, error) {
	for {
		wake := i.store.watch()
		tx, err := i.store.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
		if err != nil {
			return contracts.Batch{}, err
		}
		batch, err := readChangeBatchTx(ctx, tx, i.visibility, i.cursor)
		tx.Rollback()
		if err != nil {
			return batch, err
		}
		if batch.Through > i.cursor.Sequence {
			i.cursor.Sequence = batch.Through
			return batch, nil
		}
		select {
		case <-ctx.Done():
			return contracts.Batch{}, ctx.Err()
		case <-wake:
		}
	}
}

func readChangeBatchTx(ctx context.Context, tx *sql.Tx, visibility trust.Visibility, after contracts.Cursor) (contracts.Batch, error) {
	head, err := cursorAt(ctx, tx)
	if err != nil {
		return contracts.Batch{}, err
	}
	var floor uint64
	if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='retention_floor'").Scan(&floor); err != nil {
		return contracts.Batch{}, err
	}
	if head.Epoch != after.Epoch || head.SubscriptionRevision != after.SubscriptionRevision || after.Sequence < floor {
		return contracts.Batch{}, contracts.ErrReset
	}
	batch := contracts.Batch{Epoch: after.Epoch, SubscriptionRevision: after.SubscriptionRevision, After: after.Sequence, Through: min(head.Sequence, after.Sequence+128), Changes: []contracts.Change{}}
	if head.Sequence <= after.Sequence {
		return batch, nil
	}
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM journal WHERE seq>? AND seq<=? ORDER BY seq", batch.After, batch.Through)
	if err != nil {
		return batch, err
	}
	defer rows.Close()
	size := 0
	for rows.Next() {
		var raw []byte
		if err = rows.Scan(&raw); err != nil {
			return batch, err
		}
		var change contracts.Change
		if err = json.Unmarshal(raw, &change); err != nil {
			return batch, err
		}
		if visibility.Allows(change.Record) {
			if size+len(raw) > contracts.MaxFrame-2048 && len(batch.Changes) > 0 {
				batch.Through = change.Sequence - 1
				break
			}
			change.Record, err = projectContactLink(change.Record, visibility)
			if err != nil {
				return batch, err
			}
			batch.Changes = append(batch.Changes, change)
			size += len(raw)
		}
	}
	return batch, rows.Err()
}

func (s *Store) RetentionFloor() uint64 {
	var floor uint64
	s.db.QueryRow("SELECT value FROM meta WHERE key='retention_floor'").Scan(&floor)
	return floor
}
