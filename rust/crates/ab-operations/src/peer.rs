use crate::{Cursor, Error, Task};
pub use ab_trust::wire::messages::{Capture, IpcFrame};
use std::fmt::Debug;

pub trait ApplePeer: Debug + Send + Sync {
    fn available(&self) -> bool;
    fn exchange(&self, frame: IpcFrame) -> Task<'_, IpcFrame>;
    fn captured(
        &self,
        source: &str,
        checkpoint: &str,
        collection: &str,
        key: &str,
    ) -> Result<Cursor, Error>;
    fn capture_revision(&self) -> tokio::sync::watch::Receiver<u64>;
}
