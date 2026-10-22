package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type recordAlias struct {
	source     contracts.Source
	generation string
	collection string
	nativeKey  string
}

type recordRow struct {
	statement *sql.Stmt
	ctx       context.Context
	args      []any
	err       error
}

func (r recordRow) Scan(dest ...any) error {
	if r.err != nil {
		return r.err
	}
	return r.statement.QueryRowContext(r.ctx, r.args...).Scan(dest...)
}

type recordWriter struct {
	tx         *sql.Tx
	statements map[string]*sql.Stmt
	identities map[recordAlias]string
}

func newRecordWriter(tx *sql.Tx) *recordWriter {
	return &recordWriter{tx: tx, statements: map[string]*sql.Stmt{}, identities: map[recordAlias]string{}}
}

func (w *recordWriter) prepare(ctx context.Context, query string) (*sql.Stmt, error) {
	statement := w.statements[query]
	if statement == nil {
		var err error
		statement, err = w.tx.PrepareContext(ctx, query)
		if err != nil {
			return nil, err
		}
		w.statements[query] = statement
	}
	return statement, nil
}

func (w *recordWriter) exec(ctx context.Context, query string, args ...any) (sql.Result, error) {
	statement, err := w.prepare(ctx, query)
	if err != nil {
		return nil, err
	}
	return statement.ExecContext(ctx, args...)
}

func (w *recordWriter) row(ctx context.Context, query string, args ...any) recordRow {
	statement, err := w.prepare(ctx, query)
	return recordRow{statement: statement, ctx: ctx, args: args, err: err}
}

func (w *recordWriter) close() {
	for _, statement := range w.statements {
		statement.Close()
	}
}

func (w *recordWriter) writeRecord(ctx context.Context, r contracts.Record, seq uint64) error {
	b, err := json.Marshal(r)
	if err != nil {
		return err
	}
	_, err = w.exec(ctx, `INSERT INTO records(collection,id,version,source,deleted,payload,seq,observed_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(collection,id) DO UPDATE SET version=excluded.version,source=excluded.source,deleted=excluded.deleted,payload=excluded.payload,seq=excluded.seq,observed_at=excluded.observed_at`, r.Collection, r.ID, r.Version, r.Source, r.Deleted, b, seq, r.ObservedAt.Format(time.RFC3339Nano))
	if err != nil {
		return err
	}
	if err = w.writeReadModel(ctx, r, seq); err != nil {
		return err
	}
	return w.writeBlobReferences(ctx, "record:"+r.ID, r)
}

func (w *recordWriter) recordAt(ctx context.Context, collection, id string) (contracts.Record, error) {
	var r contracts.Record
	var b []byte
	err := w.row(ctx, "SELECT payload FROM records WHERE collection=? AND id=?", collection, id).Scan(&b)
	if err != nil {
		return r, err
	}
	err = json.Unmarshal(b, &r)
	return r, err
}

func (w *recordWriter) appendChange(ctx context.Context, c contracts.Cursor, r contracts.Record, op string) error {
	change := contracts.Change{Sequence: c.Sequence, Record: r, Op: op}
	b, err := json.Marshal(change)
	if err != nil {
		return err
	}
	_, err = w.exec(ctx, "INSERT INTO journal(seq,epoch,source,payload,committed_at) VALUES(?,?,?,?,unixepoch())", c.Sequence, c.Epoch, r.Source, b)
	return err
}

func (w *recordWriter) resolveID(ctx context.Context, c contracts.Capture, o contracts.Observation, create bool) (string, error) {
	key := recordAlias{source: c.Source, generation: c.Generation, collection: o.Collection, nativeKey: o.NativeKey}
	if id, ok := w.identities[key]; ok {
		return id, nil
	}

	var id string
	err := w.row(ctx, "SELECT record_id FROM aliases WHERE source=? AND generation=? AND collection=? AND native_key=?", c.Source, c.Generation, o.Collection, o.NativeKey).Scan(&id)
	if err == nil {
		w.identities[key] = id
		return id, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return "", err
	}
	if o.ScopedAlias != "" {
		err = w.row(ctx, "SELECT record_id FROM aliases WHERE source=? AND collection=? AND scoped_alias=? LIMIT 1", c.Source, o.Collection, o.ScopedAlias).Scan(&id)
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
	_, err = w.exec(ctx, "INSERT INTO aliases(source,generation,collection,native_key,scoped_alias,record_id) VALUES(?,?,?,?,?,?)", c.Source, c.Generation, o.Collection, o.NativeKey, o.ScopedAlias, id)
	if err == nil {
		w.identities[key] = id
	}
	return id, err
}

func (w *recordWriter) resolveReferences(ctx context.Context, c contracts.Capture, value any) (any, error) {
	switch v := value.(type) {
	case map[string]any:
		if reference, ok := v["$ref"].(map[string]any); ok {
			collection, _ := reference["collection"].(string)
			native, _ := reference["native_key"].(string)
			return w.resolveID(ctx, c, contracts.Observation{Collection: collection, NativeKey: native}, false)
		}
		for k, item := range v {
			resolved, err := w.resolveReferences(ctx, c, item)
			if err != nil {
				return nil, err
			}
			v[k] = resolved
		}
	case []any:
		for i, item := range v {
			resolved, err := w.resolveReferences(ctx, c, item)
			if err != nil {
				return nil, err
			}
			v[i] = resolved
		}
	}
	return value, nil
}

func (w *recordWriter) writeBlobReferences(ctx context.Context, owner string, r contracts.Record) error {
	if _, err := w.exec(ctx, "DELETE FROM blob_refs WHERE owner=?", owner); err != nil {
		return err
	}
	if r.Deleted {
		return nil
	}
	for _, ref := range contracts.BlobReferences(r.Data) {
		b, err := json.Marshal(ref)
		if err != nil {
			return err
		}
		if _, err = w.exec(ctx, "INSERT INTO blob_refs(owner,digest,payload) VALUES(?,?,?) ON CONFLICT(owner,digest) DO UPDATE SET payload=excluded.payload", owner, ref.Digest, b); err != nil {
			return err
		}
	}
	return nil
}
