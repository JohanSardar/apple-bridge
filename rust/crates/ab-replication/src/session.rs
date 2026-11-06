use crate::{
    Client, ConnectionState, Error, FollowStatus, Live, ReplicaState, Writer, clock, command, raw,
};
use ab_blobs::Digest;
use ab_trust::{
    Enrollment, Grant, Principal,
    wire::{framing::BlobChunk, messages::FeedFrame},
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::frame::coding::CloseCode},
};

impl<S: ReplicaState> Client<S> {
    pub(super) async fn session<T>(
        &self,
        websocket: WebSocketStream<T>,
        enrollment: &mut Enrollment,
    ) -> Result<(), Error>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let (mut sink, mut source) = websocket.split();
        let (live, mut live_rx) = mpsc::channel(32);
        let (bulk, mut bulk_rx) = mpsc::channel(4);
        let writer = Writer { live, bulk };
        let registration = command::Registration::new(self.commands.clone(), writer.clone())?;
        tokio::select! {
            result = write_messages(&mut sink, &mut live_rx, &mut bulk_rx) => result,
            result = self.read_messages(&mut source, enrollment, &writer, &registration) => result,
        }
    }
    async fn send_hello(&self, writer: &Writer, enrollment: &Enrollment) -> Result<(), Error> {
        let head = self.state.lock().await.head()?;
        let mut hello = crate::frame("Hello");
        hello.protocol = "feed/1".into();
        hello.clock_protocol = "ntp/1".into();
        hello.delegation_protocol = "delegation/1".into();
        let legacy_hello = {
            #[cfg(feature = "lab-harness")]
            {
                self.lab_legacy_hello()
            }
            #[cfg(not(feature = "lab-harness"))]
            {
                false
            }
        };
        if !legacy_hello {
            hello.features = Some(vec![raw(&"capture_atomic")?]);
        }
        hello.replica_id = enrollment.replica_id.clone();
        hello.cursor = Some(raw(&head)?);
        hello.schema_offer = Some(vec![raw(&1)?]);
        hello.credits = 4 << 20;
        writer.send(hello).await?;
        Ok(())
    }
    #[allow(clippy::too_many_lines)]
    async fn read_messages<T>(
        &self,
        source: &mut SplitStream<WebSocketStream<T>>,
        enrollment: &mut Enrollment,
        writer: &Writer,
        registration: &command::Registration,
    ) -> Result<(), Error>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        self.send_hello(writer, enrollment).await?;
        let mut replication = ReplicationProgress::<S::Install>::default();
        let mut ready = false;
        let mut synced = false;
        let mut clocked = false;
        let mut clock = clock::Probe::default();
        let mut transfers = BlobTransfers::default();
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut last = tokio::time::Instant::now();
        loop {
            let message = tokio::select! {
                _ = tick.tick() => {
                    if last.elapsed() > Duration::from_secs(15) {
                        return Err(Error::Disconnected);
                    }
                    if ready && replication.welcome.as_ref().is_some_and(|welcome| welcome.clock_protocol == "ntp/1") {
                        clock.ping(writer).await?;
                    }
                    let mut heartbeat = crate::frame("Heartbeat");
                    heartbeat.at = serde_json::from_value(serde_json::json!(jiff::Timestamp::now().to_string()))?;
                    writer.send(heartbeat).await?;
                    continue;
                },
                message = source.next() => message.ok_or(Error::Disconnected)??,
            };
            if let Message::Binary(bytes) = message {
                transfers.chunk(self.blobs.as_ref(), &bytes).await?;
                last = tokio::time::Instant::now();
                self.status.send_modify(|status| {
                    status.last_heartbeat = jiff::Timestamp::now().to_string();
                });
                continue;
            }
            let Message::Text(text) = message else {
                if let Message::Close(Some(close)) = &message
                    && close.code == CloseCode::Error
                {
                    return Err(Error::Refused(close.reason.to_string()));
                }
                if message.is_close() {
                    return Err(Error::Disconnected);
                }
                continue;
            };
            ab_trust::wire::framing::Frame::from_json(
                ab_trust::wire::framing::Protocol::Feed,
                text.as_bytes(),
            )?;
            let mut frame: FeedFrame = serde_json::from_str(&text)?;
            if frame.t == "Batch" {
                frame.batch = Some(serde_json::from_str(&text)?);
            }
            last = tokio::time::Instant::now();
            self.status
                .send_modify(|status| status.last_heartbeat = jiff::Timestamp::now().to_string());
            if frame.t == "ClockPong" {
                let sample = clock.complete(&frame, jiff::Timestamp::now())?;
                self.status
                    .send_modify(|status| status.clock_sample = Some(sample));
                clocked = true;
                if synced && !ready {
                    ready = true;
                    registration.session.ready();
                    self.status.send_modify(mark_connected);
                }
                continue;
            }
            if frame.t == "CommandResponse" {
                registration.session.deliver(&frame)?;
                continue;
            }
            if frame.t == "DelegationRequest" {
                if replication
                    .welcome
                    .as_ref()
                    .is_none_or(|welcome| welcome.delegation_protocol != "delegation/1")
                {
                    return Err(Error::Invalid);
                }
                self.answer_delegation(writer, frame).await?;
                continue;
            }
            let mut ack = crate::frame("Ack");
            ack.credits = i64::try_from(text.len()).map_err(|_| Error::Invalid)?;
            match frame.t.as_str() {
                "Welcome" | "SnapshotBegin" | "SnapshotPage" | "SnapshotEnd" | "Batch"
                | "Reset" => {
                    replication
                        .apply(self, &frame, enrollment, &mut ack)
                        .await?;
                    if frame.t == "Welcome" && frame.clock_protocol == "ntp/1" {
                        clock.ping(writer).await?;
                    }
                }
                "Heartbeat" => {
                    self.status
                        .send_modify(|status| status.gateway_head = frame.head);
                }
                "BlobAvailable" => {}
                "BlobDone" | "BlobUnavailable" => {
                    transfers.complete(self.blobs.as_ref(), &frame).await?;
                }
                _ => return Err(Error::Invalid),
            }
            if !["Heartbeat", "BlobAvailable", "BlobDone", "BlobUnavailable"]
                .contains(&frame.t.as_str())
            {
                if frame.t == "SnapshotEnd" || frame.t == "Welcome" && frame.reason == "resume" {
                    writer.send_flushed(ack).await?;
                    synced = true;
                    if !ready
                        && (clocked
                            || replication
                                .welcome
                                .as_ref()
                                .is_none_or(|welcome| welcome.clock_protocol != "ntp/1"))
                    {
                        ready = true;
                        registration.session.ready();
                        self.status.send_modify(mark_connected);
                    }
                } else {
                    writer.send(ack).await?;
                }
            }
            if replication.install.is_none() && self.blobs.is_some() {
                transfers.request_next(self, writer, &frame).await?;
            }
        }
    }
}
fn mark_connected(status: &mut FollowStatus) {
    status.state = ConnectionState::Connected;
    status.last_error.clear();
}
async fn write_messages<T>(
    sink: &mut SplitSink<WebSocketStream<T>, Message>,
    live_rx: &mut mpsc::Receiver<Live>,
    bulk_rx: &mut mpsc::Receiver<Message>,
) -> Result<(), Error>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    loop {
        let message = tokio::select! {
            biased;
            message = live_rx.recv() => message.map(|item: Live| (item.message, item.flushed)),
            message = bulk_rx.recv() => message.map(|message| (message, None)),
        };
        let Some((message, flushed)) = message else {
            return Err::<(), Error>(Error::Disconnected);
        };
        tokio::time::timeout(Duration::from_secs(10), sink.send(message))
            .await
            .map_err(|_| Error::Disconnected)??;
        if let Some(flushed) = flushed {
            let _sent = flushed.send(());
        }
    }
}
struct ReplicationProgress<Install> {
    welcome: Option<FeedFrame>,
    install: Option<Install>,
    snapshot_id: String,
    #[cfg(feature = "lab-harness")]
    pending_capture: bool,
}
impl<Install> Default for ReplicationProgress<Install> {
    fn default() -> Self {
        Self {
            welcome: None,
            install: None,
            snapshot_id: String::new(),
            #[cfg(feature = "lab-harness")]
            pending_capture: false,
        }
    }
}
impl<Install: Send> ReplicationProgress<Install> {
    async fn apply<S: ReplicaState<Install = Install>>(
        &mut self,
        client: &Client<S>,
        frame: &FeedFrame,
        enrollment: &mut Enrollment,
        ack: &mut FeedFrame,
    ) -> Result<(), Error> {
        match frame.t.as_str() {
            "Welcome" => self.welcome(client, frame, enrollment).await?,
            "SnapshotBegin" => {
                let welcome = self.welcome.as_ref().ok_or(Error::Invalid)?;
                if self.install.is_some() {
                    return Err(Error::Invalid);
                }
                self.snapshot_id.clone_from(&frame.id);
                self.install = Some(client.state.lock().await.begin_install(
                    frame,
                    &welcome.epoch,
                    welcome.subscription_revision,
                )?);
                client.status.send_modify(|status| {
                    status.state = ConnectionState::Snapshot;
                    status.snapshot_pages = 0;
                    status.snapshot_sequence = frame.s;
                });
            }
            "SnapshotPage" => {
                Client::<S>::validate_records(frame, enrollment)?;
                client
                    .state
                    .lock()
                    .await
                    .snapshot_page(self.install.as_mut().ok_or(Error::Invalid)?, frame)?;
                client
                    .status
                    .send_modify(|status| status.snapshot_pages += 1);
            }
            "SnapshotEnd" => {
                if frame.id != self.snapshot_id {
                    return Err(Error::Invalid);
                }
                let cursor = client
                    .state
                    .lock()
                    .await
                    .install_snapshot(self.install.take().ok_or(Error::Invalid)?)?;
                ack.cursor = Some(raw(&cursor)?);
                client.status.send_modify(|status| {
                    status.snapshots += 1;
                });
            }
            "Batch" => self.batch(client, frame, enrollment, ack).await?,
            "Reset" => {
                if self.welcome.is_none() || self.install.is_some() {
                    return Err(Error::Invalid);
                }
                client.state.lock().await.discard_capture()?;
                #[cfg(feature = "lab-harness")]
                {
                    if self.pending_capture {
                        client.lab_capture_discarded();
                    }
                    self.pending_capture = false;
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
}

impl<Install: Send> ReplicationProgress<Install> {
    async fn welcome<S: ReplicaState<Install = Install>>(
        &mut self,
        client: &Client<S>,
        frame: &FeedFrame,
        enrollment: &mut Enrollment,
    ) -> Result<(), Error> {
        let scope: Vec<Grant> = frame
            .scope
            .as_ref()
            .map(|value| {
                value
                    .iter()
                    .map(|reference| serde_json::from_str(reference.get()))
                    .collect()
            })
            .transpose()?
            .unwrap_or_default();
        let ceiling = if enrollment.ceiling.is_empty() {
            &enrollment.grants
        } else {
            &enrollment.ceiling
        };
        if self.welcome.is_some()
            || frame.protocol != "feed/1"
            || frame.schema != 1
            || frame.epoch.is_empty()
            || frame.subscription_revision == 0
            || !ab_trust::subset(&scope, ceiling)
        {
            return Err(Error::Invalid);
        }
        client.state.lock().await.restrict(
            &Principal {
                grants: scope.clone(),
                kind: "replica".into(),
                ..Principal::default()
            }
            .visibility(),
        )?;
        enrollment.grants = scope;
        client.authority.save_enrollment(enrollment.clone())?;
        client.status.send_modify(|status| {
            status.gateway_head = frame.head;
            status.schema = frame.schema;
            status.mode.clone_from(&frame.reason);
            if frame.reason == "resume" {
                status.resumes += 1;
            }
        });
        self.welcome = Some(frame.clone());
        Ok(())
    }
    async fn batch<S: ReplicaState<Install = Install>>(
        &mut self,
        client: &Client<S>,
        frame: &FeedFrame,
        enrollment: &Enrollment,
        ack: &mut FeedFrame,
    ) -> Result<(), Error> {
        if self.welcome.is_none() || self.install.is_some() {
            return Err(Error::Invalid);
        }
        let batch = frame.batch.as_ref().ok_or(Error::Invalid)?;
        if batch.partial
            && !self.welcome.as_ref().is_some_and(|welcome| {
                welcome
                    .features
                    .iter()
                    .flatten()
                    .any(|value| value.get() == "\"capture_atomic\"")
            })
        {
            return Err(Error::Invalid);
        }
        Client::<S>::validate_records(frame, enrollment)?;
        #[cfg(feature = "lab-harness")]
        client.lab_apply_delay().await;
        let cursor = client.state.lock().await.apply_batch(batch)?;
        if batch.partial {
            client
                .status
                .send_modify(|status| status.partial_batches += 1);
            #[cfg(feature = "lab-harness")]
            {
                self.pending_capture = true;
                client.lab_part_boundary().await?;
            }
        } else {
            #[cfg(feature = "lab-harness")]
            {
                self.pending_capture = false;
            }
        }
        if !batch.partial {
            ack.cursor = Some(raw(&cursor)?);
        }
        Ok(())
    }
}

#[derive(Default)]
struct BlobTransfers {
    active_blob: Option<Digest>,
    ignored_blob: bool,
    active_variant: bool,
    blob_queue: std::collections::VecDeque<ab_blobs::Reference>,
}
impl BlobTransfers {
    async fn chunk(
        &mut self,
        blobs: Option<&std::sync::Arc<tokio::sync::Mutex<ab_blobs::Store>>>,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let chunk = BlobChunk::decode(bytes)?;
        let digest = Digest::try_from(hex::encode(chunk.digest))?;
        if self.active_blob.as_ref() != Some(&digest) {
            return Err(Error::Invalid);
        }
        if !self.ignored_blob
            && let Some(blobs) = blobs
        {
            match blobs.lock().await.chunk(
                &digest,
                u64::try_from(chunk.offset).map_err(|_| Error::Invalid)?,
                &chunk.bytes,
            ) {
                Ok(()) => {}
                Err(ab_blobs::Error::Capacity) => self.ignored_blob = true,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
    async fn complete(
        &mut self,
        blobs: Option<&std::sync::Arc<tokio::sync::Mutex<ab_blobs::Store>>>,
        frame: &FeedFrame,
    ) -> Result<(), Error> {
        let digest = Digest::try_from(frame.digest.clone())?;
        if self.active_blob.as_ref() != Some(&digest) {
            return Err(Error::Invalid);
        }
        if frame.t == "BlobDone"
            && !self.ignored_blob
            && let Some(blobs) = blobs
        {
            blobs.lock().await.finish(&digest)?;
        } else if frame.t == "BlobUnavailable"
            && !self.ignored_blob
            && let Some(blobs) = blobs
        {
            blobs.lock().await.unavailable(&digest, &frame.reason)?;
        }
        if frame.t == "BlobUnavailable"
            && self.active_variant
            && matches!(frame.reason.as_str(), "pending" | "unavailable")
        {
            self.blob_queue
                .retain(|reference: &ab_blobs::Reference| reference.variant);
        }
        self.active_blob = None;
        self.active_variant = false;
        self.ignored_blob = false;
        Ok(())
    }
    async fn request_next<S: ReplicaState>(
        &mut self,
        client: &Client<S>,
        writer: &Writer,
        frame: &FeedFrame,
    ) -> Result<(), Error> {
        if self.blob_queue.is_empty()
            && self.active_blob.is_none()
            && [
                "Welcome",
                "Heartbeat",
                "SnapshotEnd",
                "Batch",
                "BlobAvailable",
            ]
            .contains(&frame.t.as_str())
        {
            let mut refs = client.state.lock().await.blob_references()?;
            if let Some(blobs) = &client.blobs {
                blobs.lock().await.protect_references(
                    refs.iter()
                        .map(|reference| ab_blobs::Reference {
                            digest: reference.digest.clone(),
                            size: reference.size,
                            state: reference.state.clone(),
                            ..ab_blobs::Reference::default()
                        })
                        .collect(),
                )?;
            }
            refs.sort_by(|left, right| {
                right
                    .variant
                    .cmp(&left.variant)
                    .then(left.digest.cmp(&right.digest))
            });
            let offloaded: std::collections::HashSet<String> = match &client.blobs {
                Some(blobs) => blobs
                    .lock()
                    .await
                    .entries()
                    .iter()
                    .filter(|(_, entry)| entry.state == ab_blobs::BlobState::Offloaded)
                    .map(|(digest, _)| digest.clone())
                    .collect(),
                None => std::collections::HashSet::new(),
            };
            let mut seen = std::collections::HashSet::new();
            self.blob_queue.extend(refs.into_iter().filter(|reference| {
                !reference.awaiting_variants
                    && !matches!(reference.state.as_str(), "offloaded" | "missing")
                    && !offloaded.contains(&reference.digest)
                    && seen.insert(reference.digest.clone())
            }));
        }
        if self.active_blob.is_none()
            && let Some(blobs) = &client.blobs
        {
            while let Some(reference) = self.blob_queue.pop_front() {
                let digest = Digest::try_from(reference.digest.clone())?;
                match blobs.lock().await.begin(&digest, reference.size) {
                    Ok(Some(offset)) => {
                        let mut want = crate::frame("BlobWant");
                        want.digest = digest.as_str().into();
                        want.offset = i64::try_from(offset).map_err(|_| Error::Invalid)?;
                        writer.send(want).await?;
                        self.active_blob = Some(digest);
                        self.active_variant = reference.variant;
                        break;
                    }
                    Ok(None) | Err(ab_blobs::Error::Capacity) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cursor;
    use ab_trust::wire::messages::Batch;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Debug, Default)]
    struct RecordingState {
        installs: usize,
        batches: usize,
        discards: usize,
        fail_install: bool,
    }
    impl ReplicaState for RecordingState {
        type Install = ();
        fn head(&self) -> Result<Cursor, Error> {
            Ok(Cursor {
                sequence: 9,
                ..Cursor::default()
            })
        }
        fn discard_capture(&mut self) -> Result<(), Error> {
            self.discards += 1;
            Ok(())
        }
        fn restrict(&mut self, _: &ab_trust::Visibility) -> Result<(), Error> {
            Ok(())
        }
        fn begin_install(&mut self, _: &FeedFrame, _: &str, _: u64) -> Result<(), Error> {
            Ok(())
        }
        fn snapshot_page(&mut self, (): &mut (), _: &FeedFrame) -> Result<(), Error> {
            Ok(())
        }
        fn install_snapshot(&mut self, (): ()) -> Result<Cursor, Error> {
            self.installs += 1;
            if self.fail_install {
                return Err(Error::Invalid);
            }
            self.head()
        }
        fn apply_batch(&mut self, _: &Batch) -> Result<Cursor, Error> {
            self.batches += 1;
            self.head()
        }
        fn blob_references(&self) -> Result<Vec<ab_blobs::Reference>, Error> {
            Ok(vec![])
        }
    }
    fn enrollment() -> Result<Enrollment, Error> {
        Ok(serde_json::from_value(
            serde_json::json!({"replica_id":"test", "address":"", "fingerprint":"", "grants":[]}),
        )?)
    }
    fn client(home: &std::path::Path) -> Result<Client<RecordingState>, Error> {
        Ok(Client::new(
            Arc::new(Mutex::new(RecordingState::default())),
            Arc::new(ab_trust::Authority::open(home)?),
            None,
        ))
    }
    #[derive(Debug)]
    struct UnreadableGateway;
    #[derive(Debug)]
    struct NoSnapshot;
    impl crate::server::FrozenSnapshot for NoSnapshot {
        fn begin(&self) -> FeedFrame {
            FeedFrame::default()
        }
        fn records(&self) -> &[Box<serde_json::value::RawValue>] {
            &[]
        }
        fn cursor(&self) -> Cursor {
            Cursor::default()
        }
        fn renew(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
    impl crate::server::GatewayState for UnreadableGateway {
        type Snapshot = NoSnapshot;
        type Changes = ();
        fn head(&self) -> Result<Cursor, Error> {
            Ok(Cursor {
                epoch: "synthetic".into(),
                ..Cursor::default()
            })
        }
        fn retention_floor(&self) -> Result<u64, Error> {
            Ok(0)
        }
        fn freeze_snapshot(
            &mut self,
            _: &ab_trust::Visibility,
            _: u64,
        ) -> Result<NoSnapshot, Error> {
            serde_json::from_str::<u8>("\"synthetic unreadable row\"")
                .map(|_| NoSnapshot)
                .map_err(Error::Json)
        }
        fn changes(&mut self, _: &ab_trust::Visibility, _: Cursor, _: bool) -> Result<(), Error> {
            Ok(())
        }
        fn next_changes(&mut self, (): &mut ()) -> Result<Option<Batch>, Error> {
            Ok(None)
        }
    }
    #[tokio::test]
    async fn a_gateway_that_cannot_serve_tells_the_replica_why()
    -> Result<(), Box<dyn std::error::Error>> {
        let gateway_home = tempfile::tempdir()?;
        let replica_home = tempfile::tempdir()?;
        let authority = Arc::new(ab_trust::Authority::open(gateway_home.path())?);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let code = authority.pair(
            &listener.local_addr()?.to_string(),
            "",
            vec![ab_trust::Grant::parse("system:read")?],
            600,
        )?;
        let server = Arc::new(crate::server::Server {
            delegations: Arc::new(crate::delegation::Delegations::new(authority.clone())),
            uploads: None,
            blobs: None,
            state: Arc::new(Mutex::new(UnreadableGateway)),
            authority,
        });
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let listening = tokio::spawn(server.listen(listener, stopped.clone()));
        let client = client(replica_home.path())?;
        client.authority.join(&code).await?;

        let Err(refused) = client.connect().await else {
            return Err("the gateway cannot serve".into());
        };
        assert!(
            matches!(&refused, Error::Refused(reason) if reason.contains("gateway")),
            "{refused}"
        );

        let mut status = client.status();
        let follow = client.follow(stopped);
        let seen = status.wait_for(|status| status.last_error.contains("refused"));
        tokio::select! {
            result = follow => return Err(format!("follow ended early: {result:?}").into()),
            result = tokio::time::timeout(Duration::from_secs(10), seen) => {
                let status = result??;
                assert!(status.last_error.contains("could not read"), "{}", status.last_error);
            }
        }
        stop.send(true)?;
        listening.await??;
        Ok(())
    }
    #[tokio::test]
    async fn snapshot_ack_requires_matching_id_and_successful_install()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let client = client(home.path())?;
        let mut progress = ReplicationProgress {
            install: Some(()),
            snapshot_id: "expected".into(),
            ..ReplicationProgress::default()
        };
        let mut end = crate::frame("SnapshotEnd");
        end.id = "wrong".into();
        let mut ack = crate::frame("Ack");
        let mut enrollment = enrollment()?;
        assert!(
            progress
                .apply(&client, &end, &mut enrollment, &mut ack)
                .await
                .is_err()
        );
        assert_eq!(client.state.lock().await.installs, 0);
        assert!(progress.install.is_some());
        assert!(ack.cursor.is_none());
        end.id = "expected".into();
        client.state.lock().await.fail_install = true;
        assert!(
            progress
                .apply(&client, &end, &mut enrollment, &mut ack)
                .await
                .is_err()
        );
        assert!(ack.cursor.is_none());
        progress.install = Some(());
        client.state.lock().await.fail_install = false;
        progress
            .apply(&client, &end, &mut enrollment, &mut ack)
            .await?;
        let cursor: Cursor = serde_json::from_str(ack.cursor.ok_or(Error::Invalid)?.get())?;
        assert_eq!(cursor.sequence, 9);
        Ok(())
    }
    #[tokio::test]
    async fn partial_batches_require_negotiation_and_never_ack_a_cursor()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let client = client(home.path())?;
        let mut progress = ReplicationProgress::<()>::default();
        let mut batch = crate::frame("Batch");
        batch.batch = Some(serde_json::from_value(serde_json::json!({"partial":true}))?);
        let mut ack = crate::frame("Ack");
        let mut enrollment = enrollment()?;
        assert!(
            progress
                .apply(&client, &batch, &mut enrollment, &mut ack)
                .await
                .is_err()
        );
        let mut welcome = crate::frame("Welcome");
        progress.welcome = Some(welcome.clone());
        assert!(
            progress
                .apply(&client, &batch, &mut enrollment, &mut ack)
                .await
                .is_err()
        );
        assert_eq!(client.state.lock().await.batches, 0);
        welcome.features = Some(vec![raw(&"capture_atomic")?]);
        progress.welcome = Some(welcome);
        progress
            .apply(&client, &batch, &mut enrollment, &mut ack)
            .await?;
        assert!(ack.cursor.is_none());
        assert_eq!(client.state.lock().await.batches, 1);
        progress
            .apply(&client, &crate::frame("Reset"), &mut enrollment, &mut ack)
            .await?;
        assert_eq!(client.state.lock().await.discards, 1);
        batch.batch.as_mut().ok_or(Error::Invalid)?.partial = false;
        progress
            .apply(&client, &batch, &mut enrollment, &mut ack)
            .await?;
        assert!(ack.cursor.is_some());
        progress.install = Some(());
        assert!(
            progress
                .apply(&client, &batch, &mut enrollment, &mut crate::frame("Ack"))
                .await
                .is_err()
        );
        assert_eq!(client.state.lock().await.batches, 2);
        Ok(())
    }
}
