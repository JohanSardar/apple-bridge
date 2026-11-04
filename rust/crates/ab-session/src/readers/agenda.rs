use super::{
    Arc, BTreeMap, BTreeSet, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext,
    Value, json, native_facts, reader_error,
};
use ab_native::{agenda, calendar_store, capture, contacts, reminders_enrichment};

#[derive(Debug)]
enum AgendaState {
    Calendar(agenda::Reader),
    Contacts(contacts::Reader),
    Reminders(agenda::Reader),
}

impl AgendaState {
    fn scope(&self) -> Vec<String> {
        match self {
            Self::Calendar(_) => vec!["calendar.calendar".into(), "calendar.event".into()],
            Self::Contacts(_) => vec![
                "contacts.contact".into(),
                "contacts.group".into(),
                "contacts.container".into(),
            ],
            Self::Reminders(_) => vec!["reminders.list".into(), "reminders.reminder".into()],
        }
    }

    fn knows(&self, identity: &str) -> bool {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.knows(identity),
            Self::Contacts(reader) => reader.knows(identity),
        }
    }

    fn generation(&self) -> &str {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.generation(),
            Self::Contacts(reader) => reader.generation(),
        }
    }

    fn prepare(
        &self,
        snapshot: agenda::Snapshot,
        context: &capture::Context,
    ) -> capture::Result<agenda::Pending> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.prepare(snapshot, context),
            Self::Contacts(reader) => reader.prepare(snapshot, context),
        }
    }

    fn commit(&mut self, pending: agenda::Pending) -> capture::Result<()> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.commit(pending),
            Self::Contacts(reader) => reader.commit(pending),
        }
    }

    fn unaccounted(&self, rows: &[Value]) -> Vec<Value> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.unaccounted(rows),
            Self::Contacts(reader) => reader.unaccounted(rows),
        }
    }

    fn encode(&self) -> Option<String> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => serde_json::to_string(reader).ok(),
            Self::Contacts(reader) => serde_json::to_string(reader).ok(),
        }
    }

    fn restore(&mut self, token: &str) -> bool {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => {
                match serde_json::from_str::<agenda::Reader>(token) {
                    Ok(saved) if saved.same_shape(reader) => {
                        *reader = saved;
                        true
                    }
                    _ => false,
                }
            }
            Self::Contacts(reader) => match serde_json::from_str::<contacts::Reader>(token) {
                Ok(saved) if saved.same_shape(reader) => {
                    *reader = saved;
                    true
                }
                _ => false,
            },
        }
    }
}

fn revalidation_operation(collection: &str) -> Option<&'static str> {
    Some(match collection {
        "calendar.calendar" => "agenda.calendars",
        "calendar.event" => "agenda.events",
        "reminders.list" => "agenda.reminder_lists",
        "reminders.reminder" => "agenda.reminders",
        "contacts.container" => "contacts.containers",
        "contacts.contact" => "contacts.contacts",
        "contacts.group" => "contacts.groups",
        _ => return None,
    })
}

const REVALIDATE_BYTES: usize = 180_000;

#[derive(Debug)]
struct Facts {
    generation: String,
    rows: Vec<Value>,
    absences: BTreeMap<String, agenda::Revalidation>,
    live: bool,
    blob_handles: Option<Value>,
}

#[derive(Debug)]
pub struct AgendaReader {
    source: &'static str,
    root: PathBuf,
    state: AgendaState,
    facts: Option<Facts>,
    partial: bool,
    pending: Option<agenda::Pending>,
    pull: Option<native_facts::Pull>,
    replay: bool,
    store: Option<calendar_store::Enrichment>,
    reminders_store: Option<reminders_enrichment::Enrichment>,
}

fn identity(row: &Value) -> String {
    format!(
        "{}/{}",
        row["collection"].as_str().unwrap_or_default(),
        row["native_key"].as_str().unwrap_or_default()
    )
}

impl Facts {
    fn merge(mut self, later: Self) -> Self {
        let mut rows: BTreeMap<String, Value> = self
            .rows
            .drain(..)
            .map(|row| (identity(&row), row))
            .collect();
        for row in later.rows {
            let key = identity(&row);
            self.absences.remove(&key);
            rows.insert(key, row);
        }
        for (key, disposition) in later.absences {
            rows.remove(&key);
            self.absences.insert(key, disposition);
        }
        self.rows = rows.into_values().collect();
        self.live |= later.live;
        if later.blob_handles.is_some() {
            self.blob_handles = later.blob_handles;
        }
        self
    }
}

impl AgendaReader {
    #[must_use]
    pub fn calendar(root: PathBuf) -> Self {
        Self {
            source: "calendar",
            root,
            state: AgendaState::Calendar(agenda::Reader::default()),
            facts: None,
            partial: false,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
        }
    }

    #[must_use]
    pub fn contacts(root: PathBuf) -> Self {
        Self {
            source: "contacts",
            root,
            state: AgendaState::Contacts(contacts::Reader::default()),
            facts: None,
            partial: false,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
        }
    }

    #[must_use]
    pub fn reminders(root: PathBuf) -> Self {
        Self {
            source: "reminders",
            root,
            state: AgendaState::Reminders(agenda::Reader::reminders()),
            facts: None,
            partial: false,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
        }
    }

    #[must_use]
    pub fn paged(mut self, client: Arc<native_facts::Client>, interval: Duration) -> Self {
        let operations: &'static [&'static str] = match self.state {
            AgendaState::Calendar(_) => &["agenda.calendars", "agenda.events"],
            AgendaState::Contacts(_) => &[
                "contacts.containers",
                "contacts.groups",
                "contacts.contacts",
            ],
            AgendaState::Reminders(_) => &["agenda.reminder_lists", "agenda.reminders"],
        };
        self.pull = Some(native_facts::Pull::new(
            client,
            self.source,
            operations,
            interval,
        ));
        self
    }

    fn revalidate(
        &self,
        facts: &Facts,
        fence: u64,
    ) -> Result<BTreeMap<String, agenda::Revalidation>, Error> {
        let mut verdicts = BTreeMap::new();
        let Some(pull) = self.pull.as_ref() else {
            return Ok(verdicts);
        };
        if self.state.generation() != facts.generation {
            return Ok(verdicts);
        }
        let mut grouped: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
        for previous in self.state.unaccounted(&facts.rows) {
            let collection = previous["collection"].as_str().unwrap_or_default();
            let Some(operation) = revalidation_operation(collection) else {
                continue;
            };
            grouped.entry(operation).or_default().push(json!({
                "collection": previous["collection"],
                "native_key": previous["native_key"],
                "scoped_alias": previous["scoped_alias"],
                "data": previous["data"],
            }));
        }
        for (operation, identities) in grouped {
            let mut batch: Vec<Value> = Vec::new();
            let mut bytes = 0;
            let mut batches = Vec::new();
            for identity in identities {
                let size = serde_json::to_vec(&identity)?.len() + 1;
                if !batch.is_empty()
                    && (bytes + size > REVALIDATE_BYTES
                        || batch.len() >= native_facts::MAX_REVALIDATE)
                {
                    batches.push(std::mem::take(&mut batch));
                    bytes = 0;
                }
                bytes += size;
                batch.push(identity);
            }
            if !batch.is_empty() {
                batches.push(batch);
            }
            for batch in batches {
                for verdict in pull.client().revalidate(fence, operation, &batch)? {
                    let disposition = match verdict.state {
                        native_facts::State::Present => agenda::Revalidation::Present,
                        native_facts::State::Missing => agenda::Revalidation::Absent,
                        native_facts::State::OutsideScope if self.source == "calendar" => {
                            agenda::Revalidation::Evict
                        }
                        native_facts::State::OutsideScope => return Err(Error::Invalid),
                    };
                    verdicts.insert(verdict.identity, disposition);
                }
            }
        }
        Ok(verdicts)
    }

    #[must_use]
    pub fn with_calendar_store(mut self, root: PathBuf) -> Self {
        if self.source == "calendar" {
            self.store = Some(calendar_store::Enrichment::new(root));
        }
        self
    }

    #[must_use]
    pub fn with_reminders_store(mut self, stores: PathBuf) -> Self {
        if self.source == "reminders" {
            self.reminders_store =
                Some(reminders_enrichment::Enrichment::new(Box::new(move || {
                    reminders_enrichment::paths_in(&stores)
                })));
        }
        self
    }

    fn enrich(&mut self, collected: &mut native_facts::Collected, generation: &str) {
        if let Some(store) = self.reminders_store.as_mut() {
            collected.rows = store.refresh(&collected.rows, generation);
            return;
        }
        let Some(store) = self.store.as_mut() else {
            return;
        };
        let externals: BTreeMap<String, String> = collected
            .rows
            .iter()
            .filter(|row| row["collection"] == "calendar.event")
            .filter_map(|row| {
                let alias = row["scoped_alias"].as_str()?;
                let series = alias.splitn(3, '/').nth(2)?;
                Some((
                    row["native_key"].as_str()?.to_owned(),
                    series.split('|').next().unwrap_or_default().to_owned(),
                ))
            })
            .collect();
        collected.rows = store.enrich(
            &collected.rows,
            &|native| externals.get(native).cloned().unwrap_or_default(),
            false,
        );
        for (digest, handle) in store.handles() {
            collected
                .handles
                .insert(digest.clone(), Value::String(handle.clone()));
        }
    }

    fn page(&mut self, context: &ScanContext) -> Result<(), Error> {
        let Some(pull) = self.pull.as_mut() else {
            return Ok(());
        };
        if !pull.due() {
            return Ok(());
        }
        let live = pull.live();
        let Some(mut collected) = pull.collect(context.fence)? else {
            return Ok(());
        };
        let generation = collected.generation.clone();
        self.enrich(&mut collected, &generation);
        let capture = json!({"generation": collected.generation, "observations": collected.rows, "blob_handles": collected.handles, "live": live});
        self.facts = Some(self.parse(&capture)?);
        self.partial = false;
        Ok(())
    }

    fn parse(&self, capture: &Value) -> Result<Facts, Error> {
        let generation = capture["generation"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or(Error::Invalid)?
            .to_owned();
        let observations = capture["observations"].as_array().ok_or(Error::Invalid)?;
        let scope = self.state.scope();
        let mut rows = Vec::new();
        let mut absences = BTreeMap::new();
        for observation in observations {
            let collection = observation["collection"].as_str().ok_or(Error::Invalid)?;
            let key = observation["native_key"].as_str().ok_or(Error::Invalid)?;
            if !scope.iter().any(|value| value == collection) {
                return Err(Error::Invalid);
            }
            match observation["kind"].as_str() {
                Some("upsert") => rows.push(json!({
                    "kind": "upsert",
                    "collection": collection,
                    "native_key": key,
                    "scoped_alias": observation["scoped_alias"].as_str().unwrap_or_default(),
                    "data": observation["data"],
                })),
                Some(kind @ ("absent" | "evict")) => {
                    let disposition = if kind == "evict" && self.source == "calendar" {
                        agenda::Revalidation::Evict
                    } else {
                        agenda::Revalidation::Absent
                    };
                    absences.insert(format!("{collection}/{key}"), disposition);
                }
                _ => return Err(Error::Invalid),
            }
        }
        Ok(Facts {
            generation,
            rows,
            absences,
            live: capture["live"].as_bool().unwrap_or(false),
            blob_handles: capture
                .get("blob_handles")
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                .cloned(),
        })
    }
}

impl CaptureReader for AgendaReader {
    fn source(&self) -> &'static str {
        self.source
    }

    fn owns(&self, handle: &str) -> bool {
        self.store.is_some() && handle.starts_with("calendar-attachment:")
    }

    fn handle_prefix(&self) -> Option<&'static str> {
        self.store.as_ref().map(|_| "calendar-attachment:")
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        self.store
            .as_ref()
            .ok_or(Error::Invalid)?
            .read_blob(handle, offset, length)
            .map_err(reader_error)
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        self.pull.is_some().then(|| self.state.encode()).flatten()
    }

    fn resume(&mut self, token: &str) -> bool {
        let restored = self.pull.is_some() && self.state.restore(token);
        self.replay |= restored;
        restored
    }

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        if let Some(pull) = self.pull.as_mut() {
            let partial = capture["scan_id"]
                .as_str()
                .is_some_and(|scan| !scan.is_empty())
                && capture["completion_evidence"].as_str() != Some("complete");
            if partial {
                return Ok(false);
            }
            pull.hint();
            return Ok(true);
        }
        let parsed = self.parse(capture)?;
        self.partial = capture["scan_id"]
            .as_str()
            .is_some_and(|scan| !scan.is_empty())
            && capture["completion_evidence"].as_str() != Some("complete");
        self.facts = Some(match self.facts.take() {
            Some(earlier) if earlier.generation == parsed.generation => earlier.merge(parsed),
            _ => parsed,
        });
        Ok(!self.partial)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if self.partial {
            return Ok(None);
        }
        self.page(context)?;
        let Some(facts) = self.facts.take() else {
            return Ok(None);
        };
        let revalidated = if self.pull.is_some() {
            match self.revalidate(&facts, context.fence) {
                Ok(verdicts) => verdicts,
                Err(_) => return Ok(None),
            }
        } else {
            let present: BTreeSet<String> = facts.rows.iter().map(identity).collect();
            facts
                .absences
                .iter()
                .filter(|(identity, _)| self.state.knows(identity) && !present.contains(*identity))
                .map(|(identity, disposition)| (identity.clone(), *disposition))
                .collect()
        };
        let read = capture::Context {
            observed_at: context.observed_at()?,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
        };
        let snapshot = agenda::Snapshot {
            generation: facts.generation.clone(),
            scope: self.state.scope(),
            complete: true,
            records: facts.rows,
            revalidated,
            force: self.replay,
        };
        let pending = self.state.prepare(snapshot, &read).map_err(reader_error)?;
        if pending.observations.is_empty() && self.state.generation() == facts.generation {
            return Ok(None);
        }
        let mut value = serde_json::to_value(&pending)?;
        let object = value.as_object_mut().ok_or(Error::Invalid)?;
        if facts.live {
            object.insert("live".into(), Value::Bool(true));
        }
        if let Some(handles) = facts.blob_handles {
            object.insert("blob_handles".into(), handles);
        }
        self.pending = Some(pending);
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)?;
        self.replay = false;
        Ok(())
    }
}
