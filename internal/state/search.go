package state

import (
	"context"
	"strings"
)

func (s *Store) initSearch() error {
	_, err := s.db.Exec(`CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(collection UNINDEXED,id UNINDEXED,source UNINDEXED,text);
 CREATE TRIGGER IF NOT EXISTS records_search_insert AFTER INSERT ON records WHEN NEW.deleted=0 BEGIN
 INSERT INTO records_fts(collection,id,source,text) VALUES(NEW.collection,NEW.id,NEW.source,coalesce(json_extract(NEW.payload,'$.data.text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.notes'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.plain_text'),'')); END;
 CREATE TRIGGER IF NOT EXISTS records_search_delete AFTER DELETE ON records BEGIN DELETE FROM records_fts WHERE collection=OLD.collection AND id=OLD.id; END;
 CREATE TRIGGER IF NOT EXISTS records_search_update AFTER UPDATE OF payload ON records BEGIN
 DELETE FROM records_fts WHERE collection=OLD.collection AND id=OLD.id;
 INSERT INTO records_fts(collection,id,source,text) SELECT NEW.collection,NEW.id,NEW.source,coalesce(json_extract(NEW.payload,'$.data.text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.notes'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.plain_text'),'') WHERE NEW.deleted=0; END;`)
	if err != nil {
		return err
	}
	var version string
	if s.db.QueryRow("SELECT value FROM meta WHERE key='fts_version'").Scan(&version) == nil {
		return nil
	}
	tx, err := s.db.BeginTx(context.Background(), nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.Exec(`DELETE FROM records_fts;
 INSERT INTO records_fts(collection,id,source,text) SELECT collection,id,source,coalesce(json_extract(payload,'$.data.text'),'')||' '||coalesce(json_extract(payload,'$.data.title'),'')||' '||coalesce(json_extract(payload,'$.data.notes'),'')||' '||coalesce(json_extract(payload,'$.data.plain_text'),'') FROM records WHERE deleted=0;
 INSERT INTO meta(key,value) VALUES('fts_version','1');`)
	if err != nil {
		return err
	}
	return tx.Commit()
}
func searchTerms(text string) string {
	terms := strings.Fields(text)
	for i, term := range terms {
		terms[i] = "\"" + strings.ReplaceAll(term, "\"", "\"\"") + "\""
	}
	return strings.Join(terms, " AND ")
}
