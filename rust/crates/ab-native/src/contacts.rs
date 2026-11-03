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
    pub fn prepare(&self, snapshot: Snapshot, context: &Context) -> Result<Pending> {
        self.0.prepare(snapshot, context)
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        self.0.commit(pending)
    }
}
