#![allow(clippy::unwrap_used, dead_code)]
#[path = "../../ab-native/tests/support/voicemail.rs"]
mod fixture;
mod support;
use ab_session::readers;
use serde_json::json;
use support::{Fallible, polling, receive, send, temporary, wire};

#[test]
fn oversized_voicemail_transcript_does_not_stop_other_source_captures() -> Fallible<()> {
    let root = temporary("voicemail-frame-limit")?;
    let voicemail = root.join("voicemail");
    let phone = root.join("phone");
    std::fs::create_dir_all(voicemail.join("Assets/00"))?;
    std::fs::create_dir_all(&phone)?;
    let db = rusqlite::Connection::open(voicemail.join("FaceTimeMessageStore-local.sqlitedb"))?;
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB,ZFILETYPE TEXT);")?;
    let archive = fixture::archive(&"x".repeat(40_000), 4);
    assert!(ab_native::telephony::voicemail_transcript(&archive).is_ok());
    db.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,?1,'m4a')", [archive])?;
    drop(db);
    std::fs::write(
        voicemail.join("Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a"),
        b"synthetic audio",
    )?;
    let sources = readers::from_config(&json!({"voicemail":{"root":voicemail,"staging":root.join("staging")},"phone":{"root":phone}}).to_string())?;
    let mut session = wire(sources, polling(None), json!({}))?;
    let (size, capture) = receive(&mut session.gateway)?;
    assert!(size <= 262_148);
    assert_eq!(capture["source"], "voicemail");
    let data = &capture["observations"][0]["data"];
    assert_eq!(data["transcript_state"], "unsupported");
    assert!(data.get("transcript").is_none());
    assert_eq!(data["audio_state"], "present");
    session.commit("voicemail", &capture, size)?;
    let handle = capture["blob_handles"]
        .as_object()
        .and_then(|values| values.values().next())
        .ok_or("handle")?;
    send(
        &mut session.gateway,
        &json!({"t":"BlobRead","handle":handle,"offset":0,"length":100}),
    )?;
    assert_eq!(receive(&mut session.gateway)?.1["t"], "BlobChunk");
    let db = rusqlite::Connection::open(phone.join("CallHistory.storedata"))?;
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZCALLRECORD(Z_PK INTEGER,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER);INSERT INTO ZCALLRECORD VALUES(1,'Synthetic','Synthetic',10,2,0,1,'com.apple.telephony',1);")?;
    drop(db);
    let (size, later) = receive(&mut session.gateway)?;
    assert_eq!(later["source"], "phone");
    session.commit("phone", &later, size)?;
    session.finish()?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}
