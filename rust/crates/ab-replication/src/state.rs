use crate::{Cursor, Error, ReplicaState};
use ab_state::{Replica, SnapshotInstall, State};
use ab_trust::{
    Visibility,
    wire::messages::{Batch, FeedFrame},
};
impl From<ab_state::Cursor> for Cursor {
    fn from(c: ab_state::Cursor) -> Self {
        Self {
            epoch: c.epoch,
            subscription_revision: c.subscription_revision,
            sequence: c.sequence,
        }
    }
}
impl ReplicaState for State<Replica> {
    type Install = SnapshotInstall;
    fn head(&self) -> Result<Cursor, Error> {
        State::head(self).map(Into::into).map_err(Error::Storage)
    }
    fn discard_capture(&mut self) -> Result<(), Error> {
        State::discard_capture(self).map_err(Error::Storage)
    }
    fn restrict(&mut self, visibility: &Visibility) -> Result<(), Error> {
        State::restrict(self, &visibility.to_map()).map_err(Error::Storage)
    }
    fn begin_install(
        &mut self,
        frame: &FeedFrame,
        epoch: &str,
        revision: u64,
    ) -> Result<SnapshotInstall, Error> {
        State::begin_install(self, frame, epoch, revision).map_err(Error::Storage)
    }
    fn snapshot_page(
        &mut self,
        install: &mut SnapshotInstall,
        frame: &FeedFrame,
    ) -> Result<(), Error> {
        State::snapshot_page(self, install, frame).map_err(Error::Storage)
    }
    fn install_snapshot(&mut self, install: SnapshotInstall) -> Result<Cursor, Error> {
        State::install_snapshot(self, install)
            .map(Into::into)
            .map_err(Error::Storage)
    }
    fn apply_batch(&mut self, batch: &Batch) -> Result<Cursor, Error> {
        State::apply_batch(self, batch)
            .map(Into::into)
            .map_err(Error::Storage)
    }
    fn blob_references(&self) -> Result<Vec<ab_blobs::Reference>, Error> {
        State::blob_references(self)
            .map_err(Error::Storage)?
            .into_iter()
            .map(|v| serde_json::from_value(v).map_err(Into::into))
            .collect()
    }
}
