use ab_replication::{
    Cursor, Error,
    server::{FrozenSnapshot, GatewayState, Server},
};
use ab_trust::{
    Authority, Grant, Visibility,
    wire::{
        gojson::GoTime,
        messages::{Batch, FeedFrame},
    },
};
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

#[derive(Debug)]
struct Fixture {
    started: std::time::Instant,
}
#[derive(Debug)]
struct Snapshot {
    revision: u64,
    sequence: u64,
    records: Vec<Box<serde_json::value::RawValue>>,
    digest: String,
}
impl FrozenSnapshot for Snapshot {
    fn begin(&self) -> FeedFrame {
        FeedFrame {
            t: "SnapshotBegin".into(),
            id: "fixture".into(),
            s: self.sequence,
            count: i64::try_from(self.records.len()).unwrap_or_default(),
            digest: self.digest.clone(),
            expires_at: GoTime::parse("2099-01-01T00:00:00Z").unwrap_or_default(),
            ..FeedFrame::default()
        }
    }
    fn records(&self) -> &[Box<serde_json::value::RawValue>] {
        &self.records
    }
    fn cursor(&self) -> Cursor {
        Cursor {
            epoch: "fixture-epoch".into(),
            subscription_revision: self.revision,
            sequence: self.sequence,
        }
    }
    fn renew(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
impl GatewayState for Fixture {
    fn blob_grant(
        &self,
        visibility: &Visibility,
        _: &ab_blobs::Digest,
    ) -> Result<ab_blobs::RecordGrant, Error> {
        Ok(ab_blobs::RecordGrant::from_record(visibility, &record(1)?)?)
    }
    type Snapshot = Snapshot;
    type Changes = u64;
    fn head(&self) -> Result<Cursor, Error> {
        Ok(Cursor {
            epoch: "fixture-epoch".into(),
            subscription_revision: 1,
            sequence: if self.started.elapsed().as_secs() >= 2 {
                2
            } else {
                1
            },
        })
    }
    fn retention_floor(&self) -> Result<u64, Error> {
        Ok(0)
    }
    fn freeze_snapshot(
        &mut self,
        visibility: &Visibility,
        revision: u64,
    ) -> Result<Snapshot, Error> {
        let sequence = self.head()?.sequence;
        let records: Vec<_> = [record(sequence)?]
            .into_iter()
            .filter(|r| visibility.allows(r))
            .collect();
        Ok(Snapshot {
            digest: ab_state::digest(&records),
            revision,
            sequence,
            records: records
                .iter()
                .map(serde_json::value::to_raw_value)
                .collect::<Result<_, _>>()?,
        })
    }
    fn changes(&mut self, _: &Visibility, cursor: Cursor, _: bool) -> Result<u64, Error> {
        Ok(cursor.sequence)
    }
    fn next_changes(&mut self, sequence: &mut u64) -> Result<Option<Batch>, Error> {
        let head = self.head()?.sequence;
        if *sequence >= head {
            return Ok(None);
        }
        let batch = Batch {
            epoch: "fixture-epoch".into(),
            subscription_revision: 1,
            after: *sequence,
            through: head,
            changes: Some(vec![serde_json::value::to_raw_value(
                &serde_json::json!({"sequence":head,"op":"upsert","record":record(head)?}),
            )?]),
            ..Batch::default()
        };
        *sequence = head;
        Ok(Some(batch))
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("home")?;
    let authority = Arc::new(Authority::open(&home)?);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let code = authority.pair(
        &listener.local_addr()?.to_string(),
        "",
        vec![
            Grant::parse("reminders:read")?,
            Grant::parse("messages:read")?,
        ],
        600,
    )?;
    println!("{code}");
    println!(
        "{}",
        authority.pair(
            &listener.local_addr()?.to_string(),
            "",
            vec![
                Grant::parse("reminders:read")?,
                Grant::parse("messages:read")?
            ],
            600
        )?
    );
    let mut blobs = ab_blobs::Store::open(&home, 0)?;
    blobs
        .transfer(
            &FixtureBlob,
            "fixture-handle",
            &ab_blobs::Digest::try_from(ab_trust::fingerprint(b"fixture"))?,
            7,
        )
        .await?;
    let server = Arc::new(Server {
        blobs: Some(Arc::new(Mutex::new(blobs))),
        state: Arc::new(Mutex::new(Fixture {
            started: std::time::Instant::now(),
        })),
        authority,
    });
    let (_stop, rx) = watch::channel(false);
    server.listen(listener, rx).await?;
    Ok(())
}

fn record(version: u64) -> Result<ab_trust::domain::Record, Error> {
    Ok(serde_json::from_value(
        serde_json::json!({"collection":"reminders.list","id":"lst_01ARZ3NDEKTSV4RRFFQ69G5FAV","version":version,"source":"reminders","observed_at":"2026-01-02T00:00:00Z","deleted":false,"coverage":{"fields":"","missing":null},"data":{"title":format!("fixture-{version}"), "blob":{"digest":ab_trust::fingerprint(b"fixture"), "size":7}}}),
    )?)
}

struct FixtureBlob;
impl ab_blobs::BlobRead for FixtureBlob {
    fn read(
        &self,
        handle: &str,
        offset: u64,
        length: usize,
    ) -> impl std::future::Future<Output = Result<(Vec<u8>, bool), ab_blobs::Error>> + Send {
        std::future::ready((|| {
            let offset = usize::try_from(offset).map_err(|_| ab_blobs::Error::Invalid)?;
            if handle != "fixture-handle" || offset > 7 || length == 0 {
                return Err(ab_blobs::Error::Invalid);
            }
            Ok((b"fixture"[offset..].to_vec(), true))
        })())
    }
}
