use crate::Backend;
use ab_operations::{Cursor, GatewayStore, Task};
use ab_wire::messages::{Capture, IpcFrame};
use std::collections::BTreeMap;
impl Backend for GatewayStore {
    fn next_fence(&self) -> Task<'_, u64> {
        Box::pin(async { self.next_fence() })
    }
    fn checkpoints(&self) -> Task<'_, BTreeMap<String, Box<serde_json::value::RawValue>>> {
        Box::pin(async { self.checkpoints() })
    }
    fn commit(&self, capture: Capture) -> Task<'_, (Cursor, bool)> {
        Box::pin(async move { self.commit(&capture) })
    }
    fn source_state(&self, frame: IpcFrame) -> Task<'_, ()> {
        Box::pin(async move { self.source_state(&frame) })
    }
}
