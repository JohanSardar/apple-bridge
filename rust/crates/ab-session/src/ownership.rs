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
    Rust,
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
