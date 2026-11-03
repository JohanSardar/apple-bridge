use crate::{Cursor, Error, Task};
pub use ab_trust::wire::messages::{Capture, IpcFrame};
use std::fmt::Debug;

pub trait ApplePeer: Debug + Send + Sync {
    fn available(&self) -> bool;
    fn exchange(&self, frame: IpcFrame) -> Task<'_, IpcFrame>;
    fn read_blob<'a>(
        &'a self,
        _handle: &'a str,
        _offset: i64,
        _length: i64,
    ) -> Task<'a, (Vec<u8>, bool)> {
        Box::pin(async { Err(crate::Code::Unavailable.into()) })
    }
    fn captured(
        &self,
        source: &str,
        checkpoint: &str,
        collection: &str,
        key: &str,
    ) -> Result<Cursor, Error>;
    fn capture_revision(&self) -> tokio::sync::watch::Receiver<u64>;
}
