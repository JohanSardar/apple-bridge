use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SourceId(String);

impl TryFrom<String> for SourceId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Invalid);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Producer {
    #[default]
    Swift,
    Shadow,
    Rust,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    Swift,
    Session,
}

impl Producer {
    pub fn authority(self) -> Authority {
        match self {
            Self::Swift | Self::Shadow => Authority::Swift,
            Self::Rust => Authority::Session,
        }
    }
}

#[derive(Debug, Default)]
pub struct Selection {
    sources: BTreeMap<SourceId, Producer>,
}

impl Selection {
    pub fn producer(&self, source: &SourceId) -> Producer {
        self.sources.get(source).copied().unwrap_or_default()
    }

    pub fn select_disconnected(
        &mut self,
        source: SourceId,
        producer: Producer,
        state: &ConnectionState,
    ) -> Result<(), Error> {
        if !matches!(state, ConnectionState::Disconnected) {
            return Err(Error::Conflict);
        }
        self.sources.insert(source, producer);
        Ok(())
    }

    pub fn rust_sources(&self) -> Vec<SourceId> {
        self.sources
            .iter()
            .filter(|(_, producer)| **producer == Producer::Rust)
            .map(|(source, _)| source.clone())
            .collect()
    }

    pub fn validate_routed(&self, supported: &[&str]) -> Result<(), Error> {
        if self
            .rust_sources()
            .iter()
            .any(|source| !supported.contains(&source.0.as_str()))
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    pub fn validate_proxy(&self) -> Result<(), Error> {
        if self.sources.values().any(|p| *p == Producer::Rust) {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Active {
        pending: u64,
    },
    Draining {
        pending: u64,
    },
}

impl ConnectionState {
    pub fn open(&mut self) -> Result<(), Error> {
        if !matches!(self, Self::Disconnected) {
            return Err(Error::Conflict);
        }
        *self = Self::Active { pending: 0 };
        Ok(())
    }

    pub fn capture(&mut self) -> Result<(), Error> {
        match self {
            Self::Active { pending } => {
                *pending = pending.checked_add(1).ok_or(Error::Invalid)?;
                Ok(())
            }
            _ => Err(Error::Conflict),
        }
    }

    pub fn committed(&mut self) -> Result<(), Error> {
        match self {
            Self::Active { pending } | Self::Draining { pending } if *pending > 0 => {
                *pending -= 1;
                Ok(())
            }
            _ => Err(Error::Conflict),
        }
    }

    pub fn drain(&mut self) -> Result<(), Error> {
        let Self::Active { pending } = self else {
            return Err(Error::Conflict);
        };
        *self = Self::Draining { pending: *pending };
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<(), Error> {
        if !matches!(self, Self::Draining { pending: 0 }) {
            return Err(Error::Conflict);
        }
        *self = Self::Disconnected;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationSet {
    pub scope: String,
    pub generation: String,
    pub complete: bool,
    pub records: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ShadowDiff {
    Incomplete,
    Equal,
    Different { keys: Vec<String>, metadata: bool },
}

pub fn compare_shadow(swift: &ObservationSet, rust: &ObservationSet) -> ShadowDiff {
    if !swift.complete || !rust.complete {
        return ShadowDiff::Incomplete;
    }
    let metadata = swift.scope != rust.scope || swift.generation != rust.generation;
    let keys: std::collections::BTreeSet<_> =
        swift.records.keys().chain(rust.records.keys()).collect();
    let keys: Vec<_> = keys
        .into_iter()
        .filter(|key| swift.records.get(*key) != rust.records.get(*key))
        .cloned()
        .collect();
    if !metadata && keys.is_empty() {
        ShadowDiff::Equal
    } else {
        ShadowDiff::Different { keys, metadata }
    }
}

pub trait ShadowReader {
    fn snapshot(&mut self, source: &SourceId) -> Result<ObservationSet, Error>;
}

pub fn shadow_once(
    selection: &Selection,
    source: &SourceId,
    swift: &mut impl ShadowReader,
    rust: &mut (impl ShadowReader + Send),
) -> Result<ShadowDiff, Error> {
    if selection.producer(source) != Producer::Shadow {
        return Err(Error::Conflict);
    }
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| rust.snapshot(source));
        let authoritative = swift.snapshot(source);
        let shadow = reader.join().map_err(|_| Error::WorkerPanicked)?;
        Ok(compare_shadow(&authoritative?, &shadow?))
    })
}
