use crate::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::{Value, de::IoRead, de::StreamDeserializer};
use std::io::{Read, Write};

pub struct Session<R: Read, W: Write> {
    values: StreamDeserializer<'static, IoRead<R>, Value>,
    out: W,
}
impl<R: Read, W: Write> std::fmt::Debug for Session<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}
impl<R: Read, W: Write> Session<R, W> {
    pub fn new(reader: R, out: W) -> Self {
        Self {
            values: serde_json::Deserializer::from_reader(reader).into_iter(),
            out,
        }
    }
    pub fn input<T: DeserializeOwned>(&mut self) -> Result<T> {
        match self.values.next() {
            Some(Ok(value)) => Ok(serde_json::from_value(value)?),
            Some(Err(error)) => Err(error.into()),
            None => Err(Error::Invalid("input")),
        }
    }
    pub fn signal(&mut self) -> Result<Value> {
        self.input()
    }
    pub fn emit(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.out, value)?;
        self.out.write_all(b"\n")?;
        self.out.flush()?;
        Ok(())
    }
    pub fn ready(&mut self) -> Result<()> {
        self.emit(&serde_json::json!({"ready": true}))
    }
    pub fn into_output(self) -> W {
        self.out
    }
}
