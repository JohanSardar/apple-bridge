use crate::capture::{Error, Result};
use rusqlite::{Connection, OpenFlags, Row, types::ValueRef};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Table {
    Reminder,
    List,
    Section,
    Object,
    Label,
}

impl Table {
    pub const ALL: [Self; 5] = [
        Self::Reminder,
        Self::List,
        Self::Section,
        Self::Object,
        Self::Label,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reminder => "ZREMCDREMINDER",
            Self::List => "ZREMCDBASELIST",
            Self::Section => "ZREMCDBASESECTION",
            Self::Object => "ZREMCDOBJECT",
            Self::Label => "ZREMCDHASHTAGLABEL",
        }
    }

    fn ancestor(self) -> &'static str {
        match self {
            Self::Reminder => "REMCDReminder",
            Self::List => "REMCDBaseList",
            Self::Section => "REMCDBaseSection",
            Self::Object => "REMCDObject",
            Self::Label => "REMCDHashtagLabel",
        }
    }

    fn fields(self) -> Vec<&'static str> {
        let common = ["Z_PK", "Z_ENT", "Z_OPT", "ZIDENTIFIER"];
        let extra: &[&'static str] = match self {
            Self::Reminder => &[
                "ZDACALENDARITEMUNIQUEIDENTIFIER",
                "ZEXTERNALIDENTIFIER",
                "ZFLAGGED",
                "ZPARENTREMINDER",
                "ZLIST",
                "ZCREATIONDATE",
                "ZDUEDATEDELTAALERTSDATA",
            ],
            Self::List => &[
                "ZNAME",
                "ZBADGEEMBLEM",
                "ZISGROUP",
                "ZPARENTLIST",
                "ZPARENTLIST1",
                "ZDADISPLAYORDER",
                "ZISPINNEDBYCURRENTUSER",
                "ZPINNEDDATE",
                "ZSORTINGSTYLE",
                "ZSHOULDCATEGORIZEGROCERYITEMS",
                "ZGROCERYLOCALEID",
                "ZSHARINGSTATUS",
                "ZSHAREDOWNERNAME",
                "ZSMARTLISTTYPE",
                "ZFILTERDATA",
                "ZSECTIONIDSORDERINGASDATA",
                "ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA",
                "ZREMINDERIDSMERGEABLEORDERING",
                "ZREMINDERIDSMERGEABLEORDERING_V2_JSON",
            ],
            Self::Section => &["ZDISPLAYNAME", "ZLIST", "ZSMARTLIST", "ZCREATIONDATE"],
            Self::Object => &[
                "ZREMINDER",
                "ZREMINDER1",
                "ZREMINDER2",
                "ZREMINDER3",
                "ZREMINDER4",
                "ZREMINDER5",
                "ZHASHTAGLABEL",
                "ZLIST",
            ],
            Self::Label => return vec!["Z_PK", "ZNAME"],
        };
        common.iter().chain(extra).copied().collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Integer(i64),
    Real(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub key: i64,
    pub entity: i64,
    pub version: Option<i64>,
    pub uuid: Option<String>,
    pub text: Option<String>,
}

impl Identity {
    #[must_use]
    pub fn identifier(&self) -> Option<String> {
        self.uuid.clone().or_else(|| self.text.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReminderRow {
    pub identity: Identity,
    pub calendar_item_identifier: Option<String>,
    pub flagged: Option<bool>,
    pub parent: Option<i64>,
    pub list: Option<i64>,
    pub created: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListRow {
    pub identity: Identity,
    pub emblem: Option<String>,
    pub is_group: Option<bool>,
    pub parent: Option<i64>,
    pub alternate_parent: Option<i64>,
    pub display_order: Option<f64>,
    pub pinned: Option<bool>,
    pub pinned_date: Option<f64>,
    pub sorting_style: Option<Scalar>,
    pub groceries: Option<bool>,
    pub grocery_locale: Option<String>,
    pub sharing_status: Option<i64>,
    pub shared_owner: Option<String>,
    pub smart_type: Option<Scalar>,
    pub section_order: Option<Vec<u8>>,
    pub section_membership: Option<Vec<u8>>,
    pub ordering_json: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionRow {
    pub identity: Identity,
    pub name: Option<String>,
    pub list: Option<i64>,
    pub created: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectRow {
    pub identity: Identity,
    pub reminders: [Option<i64>; 6],
    pub hashtag_label: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LabelRow {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub reminders: BTreeMap<i64, ReminderRow>,
    pub lists: BTreeMap<i64, ListRow>,
    pub sections: BTreeMap<i64, SectionRow>,
    pub objects: BTreeMap<i64, ObjectRow>,
    pub labels: BTreeMap<i64, LabelRow>,
}

impl Snapshot {
    fn keys(&self, table: Table) -> BTreeSet<i64> {
        match table {
            Table::Reminder => self.reminders.keys().copied().collect(),
            Table::List => self.lists.keys().copied().collect(),
            Table::Section => self.sections.keys().copied().collect(),
            Table::Object => self.objects.keys().copied().collect(),
            Table::Label => self.labels.keys().copied().collect(),
        }
    }

    fn clear(&mut self, table: Table) {
        match table {
            Table::Reminder => self.reminders.clear(),
            Table::List => self.lists.clear(),
            Table::Section => self.sections.clear(),
            Table::Object => self.objects.clear(),
            Table::Label => self.labels.clear(),
        }
    }

    fn remove(&mut self, table: Table, key: i64) {
        match table {
            Table::Reminder => {
                self.reminders.remove(&key);
            }
            Table::List => {
                self.lists.remove(&key);
            }
            Table::Section => {
                self.sections.remove(&key);
            }
            Table::Object => {
                self.objects.remove(&key);
            }
            Table::Label => {
                self.labels.remove(&key);
            }
        }
    }

    fn count(&self, table: Table) -> usize {
        self.keys(table).len()
    }

    fn total(&self) -> usize {
        Table::ALL.iter().map(|table| self.count(*table)).sum()
    }
}

struct Columns<'a>(&'a Row<'a>);

impl Columns<'_> {
    fn value(&self, index: usize) -> ValueRef<'_> {
        self.0.get_ref(index).unwrap_or(ValueRef::Null)
    }

    fn integer(&self, index: usize) -> Option<i64> {
        match self.value(index) {
            ValueRef::Integer(number) => Some(number),
            _ => None,
        }
    }

    fn number(&self, index: usize) -> Option<f64> {
        match self.value(index) {
            ValueRef::Integer(number) => Some(wide(number)),
            ValueRef::Real(number) => Some(number),
            _ => None,
        }
    }

    fn bool(&self, index: usize) -> Option<bool> {
        match self.integer(index) {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        }
    }

    fn text(&self, index: usize) -> Option<String> {
        match self.value(index) {
            ValueRef::Text(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    }

    fn scalar(&self, index: usize) -> Option<Scalar> {
        match self.value(index) {
            ValueRef::Integer(number) => Some(Scalar::Integer(number)),
            ValueRef::Real(number) => Some(Scalar::Real(number)),
            ValueRef::Text(bytes) => Some(Scalar::Text(String::from_utf8_lossy(bytes).into_owned())),
            _ => None,
        }
    }

    fn data(&self, index: usize) -> Option<Vec<u8>> {
        match self.value(index) {
            ValueRef::Null => None,
            ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Some(bytes.to_vec()),
            ValueRef::Integer(number) => Some(number.to_string().into_bytes()),
            ValueRef::Real(number) => Some(number.to_string().into_bytes()),
        }
    }

    fn identity(&self) -> Result<Identity> {
        let (uuid, text) = match self.value(3) {
            ValueRef::Blob(bytes) if bytes.len() == 16 => (Some(uuid_text(bytes)), None),
            ValueRef::Text(bytes) => (None, Some(String::from_utf8_lossy(bytes).into_owned())),
            _ => (None, None),
        };
        Ok(Identity {
            key: self.integer(0).ok_or(Error::Corrupt)?,
            entity: self.integer(1).ok_or(Error::Corrupt)?,
            version: self.integer(2),
            uuid,
            text,
        })
    }
}

#[allow(clippy::cast_precision_loss)]
fn wide(number: i64) -> f64 {
    number as f64
}

#[must_use]
pub fn uuid_text(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[derive(Debug)]
pub struct Store {
    connection: Connection,
    pub columns: BTreeMap<String, BTreeSet<String>>,
    pub entities: BTreeMap<i64, String>,
    entity_tables: BTreeMap<i64, Table>,
    predicates: BTreeMap<Table, String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|_| Error::Incomplete)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(25))
            .map_err(|_| Error::Incomplete)?;
        Ok(Self {
            connection,
            columns: BTreeMap::new(),
            entities: BTreeMap::new(),
            entity_tables: BTreeMap::new(),
            predicates: BTreeMap::new(),
        })
    }

    fn execute(&self, sql: &str) -> Result<()> {
        self.connection
            .execute_batch(sql)
            .map_err(|_| Error::Incomplete)
    }

    fn query(
        &self,
        sql: &str,
        binding: Option<i64>,
        mut row: impl FnMut(&Columns<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut statement = self
            .connection
            .prepare_cached(sql)
            .map_err(|_| Error::Unsupported)?;
        let mut cursor = match binding {
            Some(value) => statement.query([value]),
            None => statement.query([]),
        }
        .map_err(|_| Error::Corrupt)?;
        while let Some(found) = cursor.next().map_err(|_| Error::Corrupt)? {
            row(&Columns(found))?;
        }
        Ok(())
    }

    fn scalar(&self, sql: &str, binding: Option<i64>) -> Result<i64> {
        let mut value = 0;
        self.query(sql, binding, |row| {
            value = row.integer(0).unwrap_or(0);
            Ok(())
        })?;
        Ok(value)
    }

    fn configure(&mut self) -> Result<()> {
        self.columns.clear();
        self.entities.clear();
        self.entity_tables.clear();
        self.predicates.clear();
        let mut entities = BTreeMap::new();
        let mut parents = BTreeMap::new();
        self.query("SELECT Z_ENT,Z_NAME,Z_SUPER FROM Z_PRIMARYKEY", None, |row| {
            let id = row.integer(0).ok_or(Error::Corrupt)?;
            let name = row.text(1).ok_or(Error::Corrupt)?;
            if entities.contains_key(&id) {
                return Err(Error::Corrupt);
            }
            entities.insert(id, name);
            parents.insert(id, row.integer(2));
            Ok(())
        })?;
        self.entities = entities;
        for table in Table::ALL {
            let mut schema = BTreeSet::new();
            self.query(&format!("PRAGMA table_info({})", table.name()), None, |row| {
                if let Some(name) = row.text(1) {
                    schema.insert(name);
                }
                Ok(())
            })?;
            if !schema.contains("Z_PK") {
                return Err(Error::Unsupported);
            }
            self.columns.insert(table.name().to_owned(), schema.clone());
            if table == Table::Label {
                self.predicates.insert(table, "1".to_owned());
                for (entity, name) in &self.entities {
                    if name == table.ancestor() {
                        self.entity_tables.insert(*entity, table);
                    }
                }
                continue;
            }
            let root = self
                .entities
                .iter()
                .find(|(_, name)| name.as_str() == table.ancestor())
                .map(|(id, _)| *id)
                .ok_or(Error::Unsupported)?;
            let mut descendants: BTreeSet<i64> = BTreeSet::from([root]);
            for _ in 0..self.entities.len() {
                let next: BTreeSet<i64> = descendants
                    .iter()
                    .copied()
                    .chain(parents.iter().filter_map(|(id, parent)| {
                        parent.filter(|parent| descendants.contains(parent)).map(|_| *id)
                    }))
                    .collect();
                if next == descendants {
                    break;
                }
                descendants = next;
            }
            for entity in &descendants {
                self.entity_tables.insert(*entity, table);
            }
            let required = [
                "Z_ENT",
                "ZMARKEDFORDELETION",
                "ZCKNEEDSTOBEFETCHEDFROMCLOUD",
                "ZCKNEEDSINITIALFETCHFROMCLOUD",
            ];
            if !required.iter().all(|field| schema.contains(*field)) {
                return Err(Error::Unsupported);
            }
            let list: Vec<String> = descendants.iter().map(ToString::to_string).collect();
            self.predicates.insert(
                table,
                format!(
                    "Z_ENT IN ({}) AND COALESCE(ZMARKEDFORDELETION,0)=0 AND COALESCE(ZCKNEEDSTOBEFETCHEDFROMCLOUD,0)=0 AND COALESCE(ZCKNEEDSINITIALFETCHFROMCLOUD,0)=0",
                    list.join(",")
                ),
            );
        }
        for table in ["ATRANSACTION", "ACHANGE"] {
            let mut schema = BTreeSet::new();
            self.query(&format!("PRAGMA table_info({table})"), None, |row| {
                if let Some(name) = row.text(1) {
                    schema.insert(name);
                }
                Ok(())
            })?;
            self.columns.insert(table.to_owned(), schema);
        }
        Ok(())
    }

    fn read(&self, table: Table, key: Option<i64>, snapshot: &mut Snapshot) -> Result<()> {
        let schema = self.columns.get(table.name()).cloned().unwrap_or_default();
        let selected: Vec<&str> = table
            .fields()
            .into_iter()
            .map(|field| if schema.contains(field) { field } else { "NULL" })
            .collect();
        let predicate = self.predicates.get(&table).map_or("0", String::as_str);
        let sql = format!(
            "SELECT {} FROM {} WHERE {}{}",
            selected.join(","),
            table.name(),
            predicate,
            if key.is_some() {
                " AND Z_PK=?"
            } else {
                " ORDER BY Z_PK"
            }
        );
        self.query(&sql, key, |row| {
            match table {
                Table::Reminder => {
                    let identity = row.identity()?;
                    snapshot.reminders.insert(
                        identity.key,
                        ReminderRow {
                            calendar_item_identifier: row.text(4),
                            flagged: row.bool(6),
                            parent: row.integer(7),
                            list: row.integer(8),
                            created: row.number(9),
                            identity,
                        },
                    );
                }
                Table::List => {
                    let identity = row.identity()?;
                    snapshot.lists.insert(
                        identity.key,
                        ListRow {
                            emblem: row.text(5),
                            is_group: row.bool(6),
                            parent: row.integer(7),
                            alternate_parent: row.integer(8),
                            display_order: row.number(9),
                            pinned: row.bool(10),
                            pinned_date: row.number(11),
                            sorting_style: row.scalar(12),
                            groceries: row.bool(13),
                            grocery_locale: row.text(14),
                            sharing_status: row.integer(15),
                            shared_owner: row.text(16),
                            smart_type: row.scalar(17),
                            section_order: row.data(19),
                            section_membership: row.data(20),
                            ordering_json: row.data(22),
                            identity,
                        },
                    );
                }
                Table::Section => {
                    let identity = row.identity()?;
                    snapshot.sections.insert(
                        identity.key,
                        SectionRow {
                            name: row.text(4),
                            list: row.integer(5),
                            created: row.number(7),
                            identity,
                        },
                    );
                }
                Table::Object => {
                    let identity = row.identity()?;
                    snapshot.objects.insert(
                        identity.key,
                        ObjectRow {
                            reminders: [
                                row.integer(4),
                                row.integer(5),
                                row.integer(6),
                                row.integer(7),
                                row.integer(8),
                                row.integer(9),
                            ],
                            hashtag_label: row.integer(10),
                            identity,
                        },
                    );
                }
                Table::Label => {
                    let key = row.integer(0).ok_or(Error::Corrupt)?;
                    snapshot.labels.insert(key, LabelRow { name: row.text(1) });
                }
            }
            Ok(())
        })
    }
}

#[derive(Debug)]
pub struct ReadResult {
    pub snapshot: Snapshot,
    pub mode: &'static str,
    pub decoded_rows: usize,
    pub reminder_keys: BTreeSet<i64>,
    pub list_keys: BTreeSet<i64>,
    pub object_keys: BTreeSet<i64>,
    pub section_keys: BTreeSet<i64>,
    pub label_keys: BTreeSet<i64>,
    pub columns: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug)]
pub struct Session {
    path: std::path::PathBuf,
    store: Option<Store>,
    file_identity: (u64, u64),
    schema_version: Option<i64>,
    data_version: Option<i64>,
    history_token: Option<i64>,
    transaction_token: Option<i64>,
    cached: Snapshot,
    versions: BTreeMap<Table, BTreeMap<i64, i64>>,
    initialized: bool,
}

impl Session {
    pub fn new(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(Error::Unsupported);
        }
        Ok(Self {
            path: path.to_path_buf(),
            store: None,
            file_identity: (0, 0),
            schema_version: None,
            data_version: None,
            history_token: None,
            transaction_token: None,
            cached: Snapshot::default(),
            versions: BTreeMap::new(),
            initialized: false,
        })
    }

    pub fn read(&mut self, full: bool) -> Result<ReadResult> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&self.path)?;
        let identity = (metadata.dev(), metadata.ino());
        if identity != self.file_identity || self.store.is_none() {
            self.store = Some(Store::open(&self.path)?);
            self.file_identity = identity;
            self.schema_version = None;
            self.initialized = false;
        }
        let mut store = self.store.take().ok_or(Error::Incomplete)?;
        let outcome = self.transact(&mut store, full);
        let _ = store.execute("ROLLBACK");
        self.store = Some(store);
        outcome
    }

    fn transact(&mut self, store: &mut Store, full: bool) -> Result<ReadResult> {
        let version = store.scalar("PRAGMA data_version", None)?;
        store.execute("BEGIN")?;
        let schema = store.scalar("PRAGMA schema_version", None)?;
        let changed_schema = Some(schema) != self.schema_version;
        if changed_schema {
            store.configure()?;
        }
        let has_history = ["Z_PK", "ZENTITY", "ZENTITYPK", "ZTRANSACTIONID"]
            .iter()
            .all(|field| store.columns.get("ACHANGE").is_some_and(|set| set.contains(*field)))
            && store
                .columns
                .get("ATRANSACTION")
                .is_some_and(|set| set.contains("Z_PK"));
        let latest_history = if has_history {
            store.scalar("SELECT COALESCE(MAX(Z_PK),0) FROM ACHANGE", None)?
        } else {
            0
        };
        let latest_transaction = if has_history {
            store.scalar("SELECT COALESCE(MAX(Z_PK),0) FROM ATRANSACTION", None)?
        } else {
            0
        };
        let mut valid_token =
            has_history && self.history_token.is_some() && self.transaction_token.is_some();
        if let (Some(token), true) = (self.history_token, has_history)
            && token > 0
        {
            valid_token = store.scalar("SELECT COUNT(*) FROM ACHANGE WHERE Z_PK=?", Some(token))? == 1
                && latest_history >= token;
        }
        if let (Some(token), true) = (self.transaction_token, has_history)
            && token > 0
        {
            let anchor =
                store.scalar("SELECT COUNT(*) FROM ATRANSACTION WHERE Z_PK=?", Some(token))? == 1;
            valid_token = valid_token && anchor && latest_transaction >= token;
        }
        let mut next = self.cached.clone();
        let mut next_versions = self.versions.clone();
        let mut mode = "unchanged";
        let mut decoded = 0;
        let mut reloaded: BTreeSet<Table> = BTreeSet::new();
        let mut changes: BTreeMap<Table, BTreeSet<i64>> = BTreeMap::new();
        let needs_full = full || !self.initialized || changed_schema || (has_history && !valid_token);
        if needs_full {
            mode = "full";
            next = Snapshot::default();
            for table in Table::ALL {
                store.read(table, None, &mut next)?;
            }
            decoded = next.total();
            next_versions = if has_history {
                BTreeMap::new()
            } else {
                Self::read_versions(store)?
            };
        } else if Some(version) != self.data_version || Some(latest_history) != self.history_token {
            if let (true, Some(token)) = (valid_token, self.history_token)
                && latest_history > token
            {
                mode = "history";
                let tables = store.entity_tables.clone();
                store.query(
                    "SELECT ZENTITY,ZENTITYPK FROM ACHANGE WHERE Z_PK>? ORDER BY Z_PK",
                    Some(token),
                    |row| {
                        if let (Some(entity), Some(key)) = (row.integer(0), row.integer(1))
                            && let Some(table) = tables.get(&entity)
                        {
                            changes.entry(*table).or_default().insert(key);
                        }
                        Ok(())
                    },
                )?;
            }
            if changes.is_empty() {
                mode = "versions";
                let current = Self::read_versions(store)?;
                for table in Table::ALL {
                    let (Some(now), Some(previous)) = (current.get(&table), self.versions.get(&table))
                    else {
                        let before = next.keys(table);
                        next.clear(table);
                        store.read(table, None, &mut next)?;
                        changes.insert(table, before.union(&next.keys(table)).copied().collect());
                        reloaded.insert(table);
                        decoded += next.count(table);
                        continue;
                    };
                    let keys: BTreeSet<i64> = now.keys().chain(previous.keys()).copied().collect();
                    for key in keys {
                        if now.get(&key) != previous.get(&key) {
                            changes.entry(table).or_default().insert(key);
                        }
                    }
                }
                next_versions = current;
            }
            if mode != "full" {
                for (table, keys) in &changes {
                    if reloaded.contains(table) {
                        continue;
                    }
                    for key in keys {
                        next.remove(*table, *key);
                        store.read(*table, Some(*key), &mut next)?;
                        decoded += 1;
                    }
                }
                if mode == "history" && !next_versions.is_empty() {
                    for (table, keys) in &changes {
                        for key in keys {
                            let mut value = None;
                            if store
                                .columns
                                .get(table.name())
                                .is_some_and(|set| set.contains("Z_OPT"))
                            {
                                store.query(
                                    &format!("SELECT Z_OPT FROM {} WHERE Z_PK=?", table.name()),
                                    Some(*key),
                                    |row| {
                                        value = row.integer(0);
                                        Ok(())
                                    },
                                )?;
                            }
                            match value {
                                Some(value) => {
                                    next_versions.entry(*table).or_default().insert(*key, value);
                                }
                                None => {
                                    if let Some(rows) = next_versions.get_mut(table) {
                                        rows.remove(key);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        store.execute("COMMIT")?;
        let pick = |table: Table, current: &Snapshot, previous: &Snapshot| -> BTreeSet<i64> {
            if mode == "full" {
                current.keys(table).union(&previous.keys(table)).copied().collect()
            } else {
                changes.get(&table).cloned().unwrap_or_default()
            }
        };
        let result = ReadResult {
            reminder_keys: pick(Table::Reminder, &next, &self.cached),
            list_keys: pick(Table::List, &next, &self.cached),
            object_keys: pick(Table::Object, &next, &self.cached),
            section_keys: pick(Table::Section, &next, &self.cached),
            label_keys: pick(Table::Label, &next, &self.cached),
            snapshot: next.clone(),
            mode,
            decoded_rows: decoded,
            columns: store.columns.clone(),
        };
        self.cached = next;
        self.versions = next_versions;
        self.initialized = true;
        self.schema_version = Some(schema);
        self.data_version = Some(version);
        self.history_token = has_history.then_some(latest_history);
        self.transaction_token = has_history.then_some(latest_transaction);
        Ok(result)
    }

    fn read_versions(store: &Store) -> Result<BTreeMap<Table, BTreeMap<i64, i64>>> {
        let mut result = BTreeMap::new();
        for table in Table::ALL {
            if !store
                .columns
                .get(table.name())
                .is_some_and(|set| set.contains("Z_OPT"))
            {
                continue;
            }
            let mut rows = BTreeMap::new();
            store.query(
                &format!("SELECT Z_PK,Z_OPT FROM {}", table.name()),
                None,
                |row| {
                    if let (Some(key), Some(version)) = (row.integer(0), row.integer(1)) {
                        rows.insert(key, version);
                    }
                    Ok(())
                },
            )?;
            result.insert(table, rows);
        }
        Ok(result)
    }
}
