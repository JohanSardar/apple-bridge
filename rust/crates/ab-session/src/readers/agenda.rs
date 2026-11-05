use super::{
    Arc, BTreeMap, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Value, json,
    native_facts, reader_error,
};
use crate::calendar_lookup::{self, Candidates, Lookups, Query};
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
    live: bool,
    blob_handles: Option<Value>,
}

#[derive(Debug)]
pub struct AgendaReader {
    source: &'static str,
    root: PathBuf,
    state: AgendaState,
    facts: Option<Facts>,
    pending: Option<agenda::Pending>,
    pull: Option<native_facts::Pull>,
    replay: bool,
    store: Option<calendar_store::Enrichment>,
    reminders_store: Option<reminders_enrichment::Enrichment>,
    lookups: Option<Arc<Lookups>>,
}

impl AgendaReader {
    #[must_use]
    pub fn calendar(root: PathBuf) -> Self {
        Self {
            source: "calendar",
            root,
            state: AgendaState::Calendar(agenda::Reader::default()),
            facts: None,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
            lookups: None,
        }
    }

    #[must_use]
    pub fn contacts(root: PathBuf) -> Self {
        Self {
            source: "contacts",
            root,
            state: AgendaState::Contacts(contacts::Reader::default()),
            facts: None,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
            lookups: None,
        }
    }

    #[must_use]
    pub fn reminders(root: PathBuf) -> Self {
        Self {
            source: "reminders",
            root,
            state: AgendaState::Reminders(agenda::Reader::reminders()),
            facts: None,
            pending: None,
            pull: None,
            replay: false,
            store: None,
            reminders_store: None,
            lookups: None,
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
        let now = jiff::Zoned::now();
        for previous in self.state.unaccounted(&facts.rows) {
            let collection = previous["collection"].as_str().unwrap_or_default();
            if collection == "calendar.event"
                && let Some(lookups) = &self.lookups
            {
                let native = previous["native_key"].as_str().ok_or(Error::Invalid)?;
                let source = FactCandidates {
                    client: pull.client(),
                    fence,
                };
                let state =
                    calendar_lookup::revalidate(&source, lookups, native, &previous["data"], &now)
                        .map_err(|_| Error::Invalid)?;
                let identity = format!("{collection}/{native}");
                verdicts.insert(
                    identity,
                    match state {
                        calendar_lookup::PRESENT => agenda::Revalidation::Present,
                        calendar_lookup::MISSING => agenda::Revalidation::Absent,
                        _ => agenda::Revalidation::Evict,
                    },
                );
                continue;
            }
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

    pub fn with_lookups(mut self, path: PathBuf) -> Result<Self, Error> {
        self.lookups = Some(Arc::new(Lookups::open(Some(path))?));
        Ok(self)
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
        if let Some(lookups) = &self.lookups {
            for row in &mut collected.rows {
                let Some(fields) = row.as_object_mut() else {
                    continue;
                };
                if let Some(locator) = calendar_lookup::take_locator(fields) {
                    let key = fields
                        .get("native_key")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    lookups.remember(key, &locator)?;
                }
            }
            lookups.save()?;
        }
        self.enrich(&mut collected, &generation);
        let capture = json!({"generation": collected.generation, "observations": collected.rows, "blob_handles": collected.handles, "live": live});
        self.facts = Some(self.parse(&capture)?);
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
                _ => return Err(Error::Invalid),
            }
        }
        Ok(Facts {
            generation,
            rows,
            live: capture["live"].as_bool().unwrap_or(false),
            blob_handles: capture
                .get("blob_handles")
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                .cloned(),
        })
    }
}

struct FactCandidates<'a> {
    client: &'a native_facts::Client,
    fence: u64,
}

impl Candidates for FactCandidates<'_> {
    fn candidates(&self, query: &Query) -> Result<Vec<Value>, String> {
        self.client
            .fetch_with(
                self.fence,
                "calendar.candidates",
                &json!({"query": query.encode()}),
            )
            .map(|set| set.items)
            .map_err(|_| "unavailable".to_owned())
    }

    fn calendar_account(&self, calendar: &str) -> Result<Option<String>, String> {
        let set = self
            .client
            .fetch_with(
                self.fence,
                "calendar.account",
                &json!({"calendar": calendar}),
            )
            .map_err(|_| "unavailable".to_owned())?;
        Ok(set
            .items
            .first()
            .and_then(|item| item["account"].as_str())
            .map(str::to_owned))
    }
}

impl CaptureReader for AgendaReader {
    fn lookups(&self) -> Option<Arc<Lookups>> {
        self.lookups.clone()
    }

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

    fn refresh(&mut self) -> bool {
        match self.pull.as_mut() {
            Some(pull) => {
                pull.hint();
                true
            }
            None => false,
        }
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        self.page(context)?;
        let Some(facts) = self.facts.take() else {
            return Ok(None);
        };
        let Ok(revalidated) = self.revalidate(&facts, context.fence) else {
            if let Some(pull) = self.pull.as_mut() {
                pull.hint();
            }
            return Ok(None);
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
