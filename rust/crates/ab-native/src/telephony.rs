use crate::capture::{self, Context, Error, Pending, Result, ScanInput, Snapshot, State};
use serde_json::json;
use std::{path::Path, time::Duration};
const GENERATION: &str = "SELECT Z_UUID FROM Z_METADATA";

const CALL_COLUMNS: &[&str] = &[
    "Z_PK",
    "ZADDRESS",
    "ZNAME",
    "ZDATE",
    "ZDURATION",
    "ZORIGINATED",
    "ZANSWERED",
    "ZSERVICE_PROVIDER",
    "ZCALLTYPE",
];
const VOICEMAIL_COLUMNS: &[&str] = &[
    "Z_PK",
    "ZRECORDUUID",
    "ZFROM",
    "ZDATECREATED",
    "ZDURATION",
    "ZISREAD",
    "ZMAILBOXTYPE",
    "ZPROVIDER",
    "ZMESSAGETYPE",
];

#[must_use]
pub fn calls_readable(path: &Path) -> bool {
    Snapshot::open(path, GENERATION)
        .is_ok_and(|reader| reader.require(&[("ZCALLRECORD", CALL_COLUMNS)]).is_ok())
}

#[must_use]
pub fn voicemail_readable(path: &Path) -> bool {
    Snapshot::open(path, GENERATION).is_ok_and(|reader| {
        reader
            .require(&[("ZSTOREDMESSAGE", VOICEMAIL_COLUMNS)])
            .is_ok()
    })
}

pub fn calls(
    path: &Path,
    state: &State,
    context: &Context,
    live: bool,
    max_rows: usize,
) -> Result<Pending> {
    let reader = Snapshot::open(path, GENERATION)?;
    reader.require(&[("ZCALLRECORD", CALL_COLUMNS)])?;
    let rows=reader.query("SELECT CAST(Z_PK AS TEXT) AS id,ZADDRESS,ZNAME,ZDATE,ZDURATION,ZORIGINATED,ZANSWERED,ZSERVICE_PROVIDER,ZCALLTYPE FROM ZCALLRECORD",&[],max_rows,Duration::from_secs(5))?;
    let mut observations = Vec::new();
    for row in rows {
        let n = |k| capture::number(&row, k);
        let t = |k| capture::text(&row, k);
        let originated = n("ZORIGINATED") != 0.0;
        let answered = n("ZANSWERED") != 0.0;
        let provider = t("ZSERVICE_PROVIDER").to_lowercase();
        #[allow(clippy::float_cmp)]
        let video = n("ZCALLTYPE") == 8.0;
        let service = if provider.contains("facetime") {
            if video {
                "FaceTime video"
            } else {
                "FaceTime audio"
            }
        } else if provider.is_empty() || provider == "com.apple.telephony" {
            "cellular"
        } else {
            return Err(Error::Unsupported);
        };
        if !n("ZDURATION").is_finite() || n("ZDURATION") < 0.0 {
            return Err(Error::Corrupt);
        }
        let id = t("id");
        let data = json!({"direction":if originated{"outgoing"}else if answered{"incoming"}else{"missed"},"handle":t("ZADDRESS"),"name":t("ZNAME"),"date":capture::timestamp(n("ZDATE")+978_307_200.0)?,"duration":n("ZDURATION"),"service":service,"answered":answered});
        observations.push(capture::upsert(
            "phone.call",
            id,
            Some(format!("phone/{}/{id}", reader.generation)),
            data,
            context,
        )?);
    }
    observations.sort_by(|a, b| a.native_key.cmp(&b.native_key));
    reader.finish(GENERATION)?;
    state.prepare(ScanInput {
        source: "phone",
        generation: &reader.generation,
        records: observations,
        scope: vec!["phone.call".into()],
        coverage: json!({"fields":"complete"}),
        context,
        live,
    })
}

pub fn voicemail(
    path: &Path,
    state: &State,
    context: &Context,
    live: bool,
    max_rows: usize,
) -> Result<Pending> {
    let reader = Snapshot::open(path, GENERATION)?;
    reader.require(&[("ZSTOREDMESSAGE", VOICEMAIL_COLUMNS)])?;
    let rows=reader.query("SELECT CASE WHEN typeof(ZRECORDUUID)='blob' THEN lower(hex(ZRECORDUUID)) ELSE COALESCE(NULLIF(ZRECORDUUID,''),CAST(Z_PK AS TEXT)) END AS id,ZFROM,ZDATECREATED,ZDURATION,ZISREAD,ZMAILBOXTYPE,ZPROVIDER,ZMESSAGETYPE FROM ZSTOREDMESSAGE",&[],max_rows,Duration::from_secs(5))?;
    let mut observations = Vec::new();
    for row in rows {
        let n = |k| capture::number(&row, k);
        let t = |k| capture::text(&row, k);
        if !n("ZDURATION").is_finite() || !n("ZDATECREATED").is_finite() || n("ZDURATION") < 0.0 {
            return Err(Error::Corrupt);
        }
        let provider = t("ZPROVIDER");
        #[allow(clippy::float_cmp)]
        let video = n("ZMESSAGETYPE") == 2.0;
        let kind = if provider == "com.apple.coretelephony" {
            "carrierVoicemail"
        } else if provider.ends_with(".FaceTimeProvider") {
            if video {
                "faceTimeVideo"
            } else {
                "faceTimeAudio"
            }
        } else {
            "unknown"
        };
        let mailbox = match n("ZMAILBOXTYPE") {
            0.0 => "inbox",
            1.0 => "blocked",
            2.0 => "deleted",
            _ => "unknown",
        };
        let id = t("id");
        let data = json!({"sender":t("ZFROM"),"date":capture::timestamp(n("ZDATECREATED")+978_307_200.0)?,"duration":n("ZDURATION"),"read":n("ZISREAD")!=0.0,"mailbox":mailbox,"type":kind,"transcript_state":"unavailable","audio_state":"unavailable"});
        observations.push(capture::upsert(
            "voicemail.message",
            id,
            Some(format!("voicemail/{}/{id}", reader.generation)),
            data,
            context,
        )?);
    }
    observations.sort_by(|a, b| a.native_key.cmp(&b.native_key));
    reader.finish(GENERATION)?;
    state.prepare(ScanInput{source:"voicemail",generation:&reader.generation,records:observations,scope:vec!["voicemail.message".into()],coverage:json!({"fields":"metadata_only","transcript":"unavailable","audio":"unavailable"}),context,live})
}
