#![allow(dead_code)]
mod support;
use ab_session::readers::{CaptureReader, ScanContext};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use support::{Fallible, polling, receive, temporary, wire};

#[derive(Debug)]
struct Recordings {
    source: &'static str,
    root: PathBuf,
    committed: bool,
}
impl CaptureReader for Recordings {
    fn source(&self) -> &'static str {
        self.source
    }
    fn root(&self) -> &Path {
        &self.root
    }
    fn token(&self) -> Option<String> {
        None
    }
    fn resume(&mut self, _: &str) -> bool {
        false
    }
    fn commit(&mut self) -> Result<(), ab_session::Error> {
        self.committed = true;
        Ok(())
    }
    fn next(&mut self, _: &ScanContext) -> Result<Option<Value>, ab_session::Error> {
        if self.committed {
            return Ok(None);
        }
        let collection = if self.source == "voicemail" {
            "voicemail.message"
        } else {
            "voice_memos.recording"
        };
        let mut capture = json!({"source":self.source,"generation":"g","checkpoint":"batch","scan_id":"scan","scan_started_at":"2026-01-01T00:00:00.000Z","scope":[collection],"completion_evidence":"complete","observations":[],"blob_handles":{},"live":false});
        for index in 0..2000 {
            let digest = format!("{index:064x}");
            capture["observations"].as_array_mut().ok_or(ab_session::Error::Invalid)?.push(json!({"kind":"upsert","collection":collection,"native_key":index.to_string(),"observed_at":"2026-01-01T00:00:00.000Z","data":{"audio":{"digest":digest}}}));
            capture["blob_handles"][&digest] = json!(format!("recording:{digest}"));
        }
        Ok(Some(capture))
    }
}

#[test]
fn thousands_of_recording_handles_cross_the_real_capture_link() -> Fallible<()> {
    for source in ["voicemail", "voice_memos"] {
        let root = temporary("handle-budget")?;
        let mut readers: BTreeMap<String, Box<dyn CaptureReader>> = BTreeMap::new();
        readers.insert(
            source.into(),
            Box::new(Recordings {
                source,
                root: root.clone(),
                committed: false,
            }),
        );
        let mut session = wire(readers, polling(None), json!({}))?;
        let mut count = 0;
        let mut previous = String::new();
        while count < 2000 {
            let (bytes, capture) = receive(&mut session.gateway)?;
            assert!(bytes <= 262_148);
            assert_eq!(capture["predecessor"], previous);
            let rows = capture["observations"].as_array().ok_or("rows")?;
            assert_eq!(
                capture["blob_handles"].as_object().ok_or("handles")?.len(),
                rows.len()
            );
            count += rows.len();
            previous = capture["checkpoint"].as_str().ok_or("checkpoint")?.into();
            session.commit(source, &capture, bytes)?;
        }
        assert_eq!(count, 2000);
        assert_eq!(previous, "batch");
        session.finish()?;
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}
