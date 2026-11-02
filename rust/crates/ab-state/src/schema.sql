CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS records(collection TEXT NOT NULL,id TEXT NOT NULL,version INTEGER NOT NULL,source TEXT NOT NULL,deleted INTEGER NOT NULL,payload BLOB NOT NULL,seq INTEGER NOT NULL,observed_at TEXT NOT NULL,PRIMARY KEY(collection,id));
 CREATE INDEX IF NOT EXISTS command_approval_pending ON records(source,id) WHERE collection='system.command' AND deleted=0 AND json_extract(CAST(payload AS TEXT),'$.data.awaiting_approval')=1;
 CREATE TABLE IF NOT EXISTS aliases(source TEXT NOT NULL,generation TEXT NOT NULL,collection TEXT NOT NULL,native_key TEXT NOT NULL,scoped_alias TEXT NOT NULL,record_id TEXT NOT NULL,PRIMARY KEY(source,generation,collection,native_key));
 CREATE INDEX IF NOT EXISTS aliases_scoped ON aliases(source,collection,scoped_alias);
 CREATE INDEX IF NOT EXISTS aliases_record ON aliases(collection,record_id,source,generation,native_key);
 CREATE TABLE IF NOT EXISTS journal_commits(epoch TEXT NOT NULL,through_seq INTEGER NOT NULL,PRIMARY KEY(epoch,through_seq));
 CREATE TABLE IF NOT EXISTS pending_changes(seq INTEGER PRIMARY KEY,payload BLOB NOT NULL);
 DELETE FROM pending_changes;
 CREATE TABLE IF NOT EXISTS journal(seq INTEGER PRIMARY KEY,epoch TEXT NOT NULL,source TEXT NOT NULL,payload BLOB NOT NULL,committed_at INTEGER NOT NULL DEFAULT (unixepoch()));
 CREATE TABLE IF NOT EXISTS agent_activity(id TEXT PRIMARY KEY,principal TEXT NOT NULL,operation TEXT NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL,at INTEGER NOT NULL,count INTEGER NOT NULL,argument_bytes INTEGER NOT NULL,argument_hash TEXT NOT NULL,sequence INTEGER NOT NULL);
 CREATE INDEX IF NOT EXISTS agent_activity_at ON agent_activity(at);
 CREATE TABLE IF NOT EXISTS checkpoints(source TEXT PRIMARY KEY,generation TEXT NOT NULL,checkpoint TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS query_blobs(id TEXT PRIMARY KEY,principal TEXT,scope TEXT,payload BLOB,expires INTEGER);
 CREATE TABLE IF NOT EXISTS blob_reference_sets(owner TEXT PRIMARY KEY,payload BLOB NOT NULL);
 CREATE TABLE IF NOT EXISTS applied_batches(after_seq INTEGER PRIMARY KEY,through_seq INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS staging_records(snapshot TEXT NOT NULL,collection TEXT NOT NULL,id TEXT NOT NULL,payload BLOB NOT NULL,PRIMARY KEY(snapshot,collection,id));
 DELETE FROM staging_records;
CREATE INDEX IF NOT EXISTS calendar_timed_range ON records(julianday(json_extract(payload,'$.data.start')),julianday(json_extract(payload,'$.data.end')),id) WHERE collection='calendar.event' AND deleted=0 AND coalesce(json_extract(payload,'$.data.all_day'),0)=0;
 CREATE INDEX IF NOT EXISTS calendar_floating_range ON records(json_extract(payload,'$.data.start'),json_extract(payload,'$.data.end'),id) WHERE collection='calendar.event' AND deleted=0 AND json_extract(payload,'$.data.all_day')=1;
 CREATE INDEX IF NOT EXISTS reminder_list ON records(json_extract(payload,'$.data.list_id'),id) WHERE collection='reminders.reminder' AND deleted=0;
 CREATE INDEX IF NOT EXISTS reminder_attachment ON records(json_extract(payload,'$.data.reminder_id'),id) WHERE collection='reminders.attachment' AND deleted=0;
 CREATE TABLE IF NOT EXISTS msg_by_chat(id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,sent_at TEXT NOT NULL,unread INTEGER NOT NULL,originator TEXT NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS msg_chat_time ON msg_by_chat(chat_id,sent_at,id);
 CREATE INDEX IF NOT EXISTS msg_unread ON msg_by_chat(chat_id,sent_at,id) WHERE unread=1;
 CREATE INDEX IF NOT EXISTS msg_mention ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.has_unseen_mention')=1 AND json_extract(payload,'$.data.deleted_at') IS NULL;
 CREATE INDEX IF NOT EXISTS msg_deleted ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.deleted_at') IS NOT NULL;
 CREATE INDEX IF NOT EXISTS msg_spam ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.is_spam')=1;
 CREATE INDEX IF NOT EXISTS msg_balloon ON msg_by_chat(chat_id,json_extract(payload,'$.data.balloon.kind'),sent_at DESC,id);
 CREATE INDEX IF NOT EXISTS msg_originator ON msg_by_chat(originator);
 CREATE INDEX IF NOT EXISTS msg_thread_time ON msg_by_chat(chat_id,originator,sent_at,id);
 CREATE TABLE IF NOT EXISTS reaction_by_message(id TEXT PRIMARY KEY,message_id TEXT NOT NULL,at TEXT NOT NULL,seq INTEGER NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS reaction_message ON reaction_by_message(message_id,at,seq,id);
 CREATE TABLE IF NOT EXISTS attachment_by_message(id TEXT PRIMARY KEY,message_id TEXT NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS attachment_message ON attachment_by_message(message_id,kind,id);
 CREATE TABLE IF NOT EXISTS activity_by_chat(id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,at TEXT NOT NULL,message_id TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS activity_chat ON activity_by_chat(chat_id,at DESC,id DESC);
 CREATE INDEX IF NOT EXISTS activity_message ON activity_by_chat(message_id);
 CREATE TABLE IF NOT EXISTS chat_summary(id TEXT PRIMARY KEY,last_activity_at TEXT NOT NULL DEFAULT '',unread_count INTEGER NOT NULL DEFAULT 0);
 CREATE INDEX IF NOT EXISTS chat_activity ON chat_summary(last_activity_at DESC,id);
 CREATE TRIGGER IF NOT EXISTS rm_msg_insert AFTER INSERT ON msg_by_chat BEGIN
 INSERT INTO chat_summary(id,unread_count) VALUES(new.chat_id,new.unread) ON CONFLICT(id) DO UPDATE SET unread_count=unread_count+new.unread;
 END;
 CREATE TRIGGER IF NOT EXISTS rm_msg_delete AFTER DELETE ON msg_by_chat BEGIN
 UPDATE chat_summary SET unread_count=unread_count-old.unread WHERE id=old.chat_id;
 END;
 CREATE TRIGGER IF NOT EXISTS rm_activity_insert AFTER INSERT ON activity_by_chat BEGIN
 INSERT INTO chat_summary(id,last_activity_at) VALUES(new.chat_id,new.at) ON CONFLICT(id) DO UPDATE SET last_activity_at=coalesce((SELECT at FROM activity_by_chat WHERE chat_id=new.chat_id ORDER BY at DESC,id DESC LIMIT 1),'');
 END;
 CREATE TRIGGER IF NOT EXISTS rm_activity_delete AFTER DELETE ON activity_by_chat BEGIN
 UPDATE chat_summary SET last_activity_at=coalesce((SELECT at FROM activity_by_chat WHERE chat_id=old.chat_id ORDER BY at DESC,id DESC LIMIT 1),'') WHERE id=old.chat_id;
 END;
CREATE TABLE IF NOT EXISTS handle_address(id TEXT PRIMARY KEY,address TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS handle_address_value ON handle_address(address,id);
 CREATE TABLE IF NOT EXISTS chat_participant(chat_id TEXT NOT NULL,handle_id TEXT NOT NULL,PRIMARY KEY(chat_id,handle_id));
 CREATE INDEX IF NOT EXISTS chat_participant_handle ON chat_participant(handle_id,chat_id);
CREATE TABLE IF NOT EXISTS blob_refs(owner TEXT NOT NULL,digest TEXT NOT NULL,payload BLOB NOT NULL,PRIMARY KEY(owner,digest));
CREATE INDEX IF NOT EXISTS blob_refs_digest ON blob_refs(digest);
CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(collection UNINDEXED,id UNINDEXED,source UNINDEXED,text);
CREATE TABLE IF NOT EXISTS records_fts_map(fts_rowid INTEGER PRIMARY KEY,collection TEXT NOT NULL,id TEXT NOT NULL,UNIQUE(collection,id));
CREATE TRIGGER IF NOT EXISTS records_search_insert AFTER INSERT ON records WHEN NEW.deleted=0 BEGIN
 INSERT INTO records_fts_map(collection,id) SELECT NEW.collection,NEW.id WHERE NOT EXISTS(SELECT 1 FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id);
 INSERT INTO records_fts(rowid,collection,id,source,text) SELECT fts_rowid,NEW.collection,NEW.id,NEW.source,coalesce(json_extract(NEW.payload,'$.data.subject'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.from'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.to'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.snippet'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.notes'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.plain_text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.given_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.middle_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.family_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.nickname'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.organization'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.job_title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.filename'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.balloon.link.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.balloon.app.caption'),'') FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id; END;
 CREATE TRIGGER IF NOT EXISTS records_search_delete AFTER DELETE ON records BEGIN
 DELETE FROM records_fts WHERE rowid=(SELECT fts_rowid FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id);
 DELETE FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id; END;
 CREATE TRIGGER IF NOT EXISTS records_search_update AFTER UPDATE OF payload ON records BEGIN
 INSERT INTO records_fts_map(collection,id) SELECT NEW.collection,NEW.id WHERE NEW.deleted=0 AND NOT EXISTS(SELECT 1 FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id);
 DELETE FROM records_fts WHERE rowid=(SELECT fts_rowid FROM records_fts_map WHERE collection=OLD.collection AND id=OLD.id);
 INSERT INTO records_fts(rowid,collection,id,source,text) SELECT fts_rowid,NEW.collection,NEW.id,NEW.source,coalesce(json_extract(NEW.payload,'$.data.subject'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.from'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.to'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.snippet'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.notes'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.plain_text'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.given_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.middle_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.family_name'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.nickname'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.organization'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.job_title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.filename'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.balloon.link.title'),'')||' '||coalesce(json_extract(NEW.payload,'$.data.balloon.app.caption'),'') FROM records_fts_map WHERE collection=NEW.collection AND id=NEW.id AND NEW.deleted=0; END;
