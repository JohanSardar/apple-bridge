use crate::{
    Cursor, Error, raw,
    server::{FrozenSnapshot, GatewayState},
};
use ab_state::{Gateway, State};
use ab_trust::{
    Visibility,
    wire::messages::{Batch, FeedFrame},
};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug)]
pub struct StateAdapter(pub Arc<Mutex<State<Gateway>>>);
impl StateAdapter {
    fn lock(&self) -> Result<MutexGuard<'_, State<Gateway>>, Error> {
        self.0
            .lock()
            .map_err(|_| Error::State("gateway state mutex poisoned".into()))
    }
}
fn storage(error: ab_state::Error) -> Error {
    match error {
        ab_state::Error::Contract(code) => Error::Contract(code),
        other => Error::Storage(other),
    }
}
#[derive(Debug)]
pub struct Snapshot {
    state: StateAdapter,
    frozen: ab_state::FrozenSnapshot,
    records: Vec<Box<serde_json::value::RawValue>>,
    begin: FeedFrame,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        blocking(|| {
            if let Ok(mut state) = self.state.lock() {
                let _ = state.close_snapshot(&self.frozen.id);
            }
        });
    }
}
impl FrozenSnapshot for Snapshot {
    fn begin(&self) -> FeedFrame {
        self.begin.clone()
    }
    fn records(&self) -> &[Box<serde_json::value::RawValue>] {
        &self.records
    }
    fn cursor(&self) -> Cursor {
        self.frozen.cursor.clone().into()
    }
    fn renew(&mut self) -> Result<(), Error> {
        self.state
            .lock()?
            .renew_snapshot(&self.frozen.id)
            .map_err(storage)
    }
}
impl GatewayState for StateAdapter {
    type Snapshot = Snapshot;
    type Changes = ab_state::Changes;
    fn head(&self) -> Result<Cursor, Error> {
        blocking(|| self.lock()?.head().map(Into::into).map_err(storage))
    }
    fn retention_floor(&self) -> Result<u64, Error> {
        blocking(|| self.lock()?.retention_floor().map_err(storage))
    }
    fn freeze_snapshot(
        &mut self,
        visibility: &Visibility,
        revision: u64,
    ) -> Result<Snapshot, Error> {
        blocking(|| {
            let frozen = self
                .lock()?
                .freeze_snapshot(&visibility.to_map(), revision)
                .map_err(storage)?;
            let result = (|| {
                let records = frozen.records.iter().map(raw).collect::<Result<_, _>>()?;
                let begin = FeedFrame {
                    t: "SnapshotBegin".into(),
                    id: frozen.id.clone(),
                    s: frozen.cursor.sequence,
                    count: i64::try_from(frozen.count).map_err(|_| Error::Invalid)?,
                    digest: frozen.digest.clone(),
                    expires_at: ab_trust::wire::gojson::GoTime::parse(&frozen.expires_at)
                        .map_err(|_| Error::Invalid)?,
                    ..FeedFrame::default()
                };
                Ok((records, begin))
            })();
            match result {
                Ok((records, begin)) => Ok(Snapshot {
                    state: self.clone(),
                    frozen,
                    records,
                    begin,
                }),
                Err(error) => {
                    self.lock()?.close_snapshot(&frozen.id).map_err(storage)?;
                    Err(error)
                }
            }
        })
    }
    fn changes(
        &mut self,
        visibility: &Visibility,
        cursor: Cursor,
        atomic: bool,
    ) -> Result<ab_state::Changes, Error> {
        blocking(|| {
            let cursor = ab_state::Cursor {
                epoch: cursor.epoch,
                subscription_revision: cursor.subscription_revision,
                sequence: cursor.sequence,
            };
            let state = self.lock()?;
            if atomic {
                state.feed_changes(visibility.to_map(), cursor)
            } else {
                state.legacy_changes(visibility.to_map(), cursor)
            }
            .map_err(storage)
        })
    }
    fn next_changes(&mut self, iterator: &mut ab_state::Changes) -> Result<Option<Batch>, Error> {
        blocking(|| self.lock()?.next_changes(iterator).map_err(storage))
    }
    fn blob_grant(
        &self,
        visibility: &Visibility,
        digest: &ab_blobs::Digest,
    ) -> Result<ab_blobs::RecordGrant, Error> {
        blocking(|| {
            let state = self.lock()?;
            let mut after = String::new();
            loop {
                let page = state
                    .read(
                        &visibility.to_map(),
                        &ab_state::ReadQuery {
                            after,
                            limit: 1000,
                            ..Default::default()
                        },
                    )
                    .map_err(storage)?;
                for record in page.records {
                    let grant = ab_blobs::RecordGrant::from_record(visibility, &record)?;
                    if grant.allows(digest) {
                        return Ok(grant);
                    }
                }
                if page.next.is_empty() {
                    return Err(Error::Invalid);
                }
                after = page.next;
            }
        })
    }
}

fn blocking<T>(work: impl FnOnce() -> T) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(work)
    } else {
        work()
    }
}
#[derive(Debug)]
pub struct ProtectionAdapter(pub Arc<tokio::sync::Mutex<ab_blobs::Store>>);
impl ab_state::BlobProtection for ProtectionAdapter {
    fn delta(&self, references: &ab_state::ReferenceSet, committed: bool) -> ab_state::Result<()> {
        let updates = references
            .iter()
            .map(|(owner, refs)| {
                Ok((
                    owner.clone(),
                    refs.iter()
                        .cloned()
                        .map(serde_json::from_value)
                        .collect::<Result<Vec<ab_blobs::Reference>, _>>()?,
                ))
            })
            .collect::<ab_state::Result<std::collections::BTreeMap<_, _>>>()?;
        blocking(|| {
            self.0
                .blocking_lock()
                .update_protection(&updates, committed)
        })
        .map_err(|error| match error {
            ab_blobs::Error::Io(error) => ab_state::Error::Io(error),
            ab_blobs::Error::Sqlite(error) => ab_state::Error::Sqlite(error),
            _ => ab_state::Error::Contract(ab_trust::domain::ErrorCode::Unavailable),
        })
    }
}
