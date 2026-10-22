package state

import (
	"context"
	"database/sql"
	"fmt"
	"runtime"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

const searchVersion = "4"

func searchTextSQL(payload string) string {
	fields := []string{"text", "title", "notes", "plain_text", "given_name", "middle_name", "family_name", "nickname", "organization", "job_title", "filename", "balloon.link.title", "balloon.app.caption"}
	parts := make([]string, len(fields))
	for i, field := range fields {
		parts[i] = fmt.Sprintf("coalesce(json_extract(%s,'$.data.%s'),'')", payload, field)
	}
	return strings.Join(parts, "||' '||")
}

func searchTriggers(table string) string {
	text := searchTextSQL("NEW.payload")
	return fmt.Sprintf(`CREATE TRIGGER records_search_insert AFTER INSERT ON records WHEN NEW.deleted=0 BEGIN
 INSERT INTO records_fts_map(collection,id) SELECT NEW.collection,NEW.id WHERE NOT EXISTS(SELECT 1 FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id);
 INSERT INTO %[1]s(rowid,collection,id,source,text) SELECT fts_rowid,NEW.collection,NEW.id,NEW.source,%[2]s FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id; END;
 CREATE TRIGGER records_search_delete AFTER DELETE ON records BEGIN
 DELETE FROM %[1]s WHERE rowid=(SELECT fts_rowid FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id);
 DELETE FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id; END;
 CREATE TRIGGER records_search_update AFTER UPDATE OF payload ON records BEGIN
 INSERT INTO records_fts_map(collection,id) SELECT NEW.collection,NEW.id WHERE NEW.deleted=0 AND NOT EXISTS(SELECT 1 FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id);
 DELETE FROM %[1]s WHERE rowid=(SELECT fts_rowid FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id);
 INSERT INTO %[1]s(rowid,collection,id,source,text) SELECT fts_rowid,NEW.collection,NEW.id,NEW.source,%[2]s FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id AND NEW.deleted=0; END;`, table, text)
}

const dropSearchTriggers = `DROP TRIGGER IF EXISTS records_search_insert; DROP TRIGGER IF EXISTS records_search_update; DROP TRIGGER IF EXISTS records_search_delete;`

func (s *Store) initSearch() (bool, error) {
	s.write.Lock()
	defer s.write.Unlock()
	var version string
	err := s.db.QueryRow("SELECT value FROM meta WHERE key='fts_version'").Scan(&version)
	if err != nil && err != sql.ErrNoRows {
		return false, err
	}
	if version == searchVersion {
		return false, nil
	}
	tx, err := s.db.BeginTx(context.Background(), nil)
	if err != nil {
		return false, err
	}
	defer tx.Rollback()
	if _, err = tx.Exec("UPDATE meta SET value=value WHERE key='fts_version'"); err != nil {
		return false, err
	}
	version = ""
	err = tx.QueryRow("SELECT value FROM meta WHERE key='fts_version'").Scan(&version)
	if err != nil && err != sql.ErrNoRows {
		return false, err
	}
	if version == searchVersion {
		return false, tx.Commit()
	}
	if _, err = tx.Exec(dropSearchTriggers); err != nil {
		return false, err
	}
	if version != "building"+searchVersion {
		if _, err = tx.Exec("DROP TABLE IF EXISTS records_fts_next; DROP TABLE IF EXISTS records_fts_map"); err != nil {
			return false, err
		}
	}
	_, err = tx.Exec(`CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(collection UNINDEXED,id UNINDEXED,source UNINDEXED,text);
 CREATE TABLE IF NOT EXISTS records_fts_map(fts_rowid INTEGER PRIMARY KEY,collection TEXT NOT NULL,id TEXT NOT NULL,UNIQUE(collection,id));`)
	if err != nil {
		return false, err
	}
	var populated bool
	if err = tx.QueryRow("SELECT EXISTS(SELECT 1 FROM records LIMIT 1)").Scan(&populated); err != nil {
		return false, err
	}
	table := "records_fts"
	ready := searchVersion
	if populated || version == "building"+searchVersion {
		table = "records_fts_next"
		ready = "building" + searchVersion
		if _, err = tx.Exec("CREATE VIRTUAL TABLE IF NOT EXISTS records_fts_next USING fts5(collection UNINDEXED,id UNINDEXED,source UNINDEXED,text)"); err != nil {
			return false, err
		}
	}
	if _, err = tx.Exec(searchTriggers(table)); err != nil {
		return false, err
	}
	if _, err = tx.Exec("INSERT INTO meta(key,value) VALUES('fts_version',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", ready); err != nil {
		return false, err
	}
	return ready != searchVersion, tx.Commit()
}

func (s *Store) startSearchRebuild() {
	ctx, cancel := context.WithCancel(context.Background())
	s.searchCancel = cancel
	s.searchDone = make(chan struct{})
	go func() {
		defer close(s.searchDone)
		if s.rebuildSearch(ctx) == nil {
			s.signal()
		}
	}()
}

func (s *Store) rebuildSearch(ctx context.Context) error {
	collection, id := "", ""
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		nextCollection, nextID, done, err := s.rebuildSearchBatch(ctx, collection, id)
		if err != nil || done {
			return err
		}
		collection, id = nextCollection, nextID
		runtime.Gosched()
	}
}

func (s *Store) rebuildSearchBatch(ctx context.Context, collection, id string) (string, string, bool, error) {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return "", "", false, err
	}
	defer tx.Rollback()
	var version string
	if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='fts_version'").Scan(&version); err != nil {
		return "", "", false, err
	}
	if version == searchVersion {
		return collection, id, true, nil
	}
	rows, err := tx.QueryContext(ctx, "SELECT collection,id FROM records WHERE (collection,id)>(?,?) ORDER BY collection,id LIMIT 512", collection, id)
	if err != nil {
		return "", "", false, err
	}
	nextCollection, nextID := collection, id
	found := false
	for rows.Next() {
		if err = rows.Scan(&nextCollection, &nextID); err != nil {
			break
		}
		found = true
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return "", "", false, err
	}
	if !found {
		_, err = tx.ExecContext(ctx, dropSearchTriggers+"DROP TABLE records_fts; ALTER TABLE records_fts_next RENAME TO records_fts;"+searchTriggers("records_fts"))
		if err == nil {
			_, err = tx.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='fts_version'", searchVersion)
		}
		if err == nil {
			err = tx.Commit()
		}
		return nextCollection, nextID, true, err
	}
	bounds := "r.deleted=0 AND (r.collection,r.id)>(?,?) AND (r.collection,r.id)<=(?,?)"
	args := []any{collection, id, nextCollection, nextID}
	_, err = tx.ExecContext(ctx, "INSERT OR IGNORE INTO records_fts_map(collection,id) SELECT r.collection,r.id FROM records r WHERE "+bounds, args...)
	if err == nil {
		_, err = tx.ExecContext(ctx, "INSERT OR REPLACE INTO records_fts_next(rowid,collection,id,source,text) SELECT m.fts_rowid,r.collection,r.id,r.source,"+searchTextSQL("r.payload")+" FROM records r JOIN records_fts_map m ON m.collection=r.collection AND m.id=r.id WHERE "+bounds, args...)
	}
	if err == nil {
		err = tx.Commit()
	}
	return nextCollection, nextID, false, err
}

func requireSearchReady(ctx context.Context, tx *sql.Tx) error {
	var version string
	if err := tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='fts_version'").Scan(&version); err != nil {
		return err
	}
	if version != searchVersion {
		return &contracts.Error{Code: "unavailable", Message: "search is preparing", Retryable: true}
	}
	return nil
}

func searchTerms(text string) string {
	terms := strings.Fields(text)
	for i, term := range terms {
		terms[i] = "\"" + strings.ReplaceAll(term, "\"", "\"\"") + "\""
	}
	return strings.Join(terms, " AND ")
}
