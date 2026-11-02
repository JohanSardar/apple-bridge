use ab_replication::{Cursor, Error, ReplicaState};
use ab_wire::messages::{Batch, FeedFrame};
use std::sync::{Arc, Mutex, MutexGuard};
#[derive(Debug)]
pub struct StateAdapter(pub Arc<Mutex<ab_state::State<ab_state::Replica>>>);
impl StateAdapter {
    fn lock(&self) -> Result<MutexGuard<'_, ab_state::State<ab_state::Replica>>, Error> {
        self.0
            .lock()
            .map_err(|_| Error::State("state lock poisoned".into()))
    }
}
fn state_error(error: ab_state::Error) -> Error {
    Error::State(error.to_string())
}
fn cursor(value: ab_state::Cursor) -> Cursor {
    Cursor {
        epoch: value.epoch,
        subscription_revision: value.subscription_revision,
        sequence: value.sequence,
    }
}
impl ReplicaState for StateAdapter {
    type Install = ab_state::SnapshotInstall;
    fn head(&self) -> Result<Cursor, Error> {
        Ok(cursor(self.lock()?.head().map_err(state_error)?))
    }
    fn discard_capture(&mut self) -> Result<(), Error> {
        self.lock()?.discard_capture().map_err(state_error)
    }
    fn restrict(&mut self, visibility: &ab_trust::Visibility) -> Result<(), Error> {
        let mut state = self.lock()?;
        state
            .restrict(&visibility_map(visibility))
            .map_err(state_error)
    }
    fn begin_install(
        &mut self,
        frame: &FeedFrame,
        epoch: &str,
        revision: u64,
    ) -> Result<Self::Install, Error> {
        self.lock()?
            .begin_install(frame, epoch, revision)
            .map_err(state_error)
    }
    fn snapshot_page(
        &mut self,
        install: &mut Self::Install,
        frame: &FeedFrame,
    ) -> Result<(), Error> {
        self.lock()?
            .snapshot_page(install, frame)
            .map_err(state_error)
    }
    fn install_snapshot(&mut self, install: Self::Install) -> Result<Cursor, Error> {
        Ok(cursor(
            self.lock()?
                .install_snapshot(install)
                .map_err(state_error)?,
        ))
    }
    fn apply_batch(&mut self, batch: &Batch) -> Result<Cursor, Error> {
        Ok(cursor(
            self.lock()?.apply_batch(batch).map_err(state_error)?,
        ))
    }
    fn blob_references(&self) -> Result<Vec<ab_blobs::Reference>, Error> {
        self.lock()?
            .blob_references()
            .map_err(state_error)?
            .iter()
            .map(|value| serde_json::from_value(value.clone()).map_err(Error::from))
            .collect()
    }
}
fn visibility_map(visibility: &ab_trust::Visibility) -> ab_state::Visibility {
    let mut result = ab_state::Visibility::new();
    for source in &visibility.sources {
        result.insert(source.clone(), true);
    }
    for principal in &visibility.principals {
        result.insert(format!("principal:{principal}"), true);
    }
    if visibility.commands_all {
        result.insert("commands:all".into(), true);
    }
    if visibility.all {
        for source in [
            "location",
            "audio",
            "calendar",
            "capture",
            "contacts",
            "files",
            "focus",
            "health",
            "iphone",
            "mac",
            "mail",
            "maps",
            "messages",
            "notes",
            "pdf",
            "phone",
            "print",
            "reminders",
            "safari",
            "shortcuts",
            "speech",
            "system",
            "usage",
            "vision",
            "voice_memos",
            "voicemail",
        ] {
            result.insert(source.into(), true);
        }
    }
    result
}
