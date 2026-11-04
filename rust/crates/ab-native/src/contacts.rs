use crate::agenda::{Pending, Snapshot};
use crate::capture::{Context, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reader(crate::agenda::Reader);

impl Default for Reader {
    fn default() -> Self {
        Self(crate::agenda::Reader::new(
            "contacts",
            &["contacts.contact", "contacts.group", "contacts.container"],
        ))
    }
}

impl Reader {
    #[must_use]
    pub fn knows(&self, identity: &str) -> bool {
        self.0.knows(identity)
    }

    #[must_use]
    pub fn generation(&self) -> &str {
        self.0.generation()
    }

    #[must_use]
    pub fn same_shape(&self, other: &Self) -> bool {
        self.0.same_shape(&other.0)
    }

    #[must_use]
    pub fn unaccounted(&self, rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
        self.0.unaccounted(rows)
    }

    pub fn prepare(&self, snapshot: Snapshot, context: &Context) -> Result<Pending> {
        self.0.prepare(snapshot, context)
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        self.0.commit(pending)
    }
}
