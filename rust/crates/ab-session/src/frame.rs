use crate::Error;
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
};

pub const MAX_FRAME: usize = 262_144;

fn read(stream: &mut UnixStream, large: &[&str]) -> Result<(Vec<u8>, Value), Error> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header)).map_err(|_| Error::Invalid)?;
    if size == 0 || size > 1_114_112 {
        return Err(Error::Invalid);
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    if !value.is_object()
        || value["t"].as_str().is_none()
        || (size > MAX_FRAME && !value["t"].as_str().is_some_and(|t| large.contains(&t)))
    {
        return Err(Error::Invalid);
    }
    let mut frame = header.to_vec();
    frame.extend(bytes);
    Ok((frame, value))
}

pub fn read_frame(stream: &mut UnixStream) -> Result<(Vec<u8>, Value), Error> {
    read(stream, &["QueryResult"])
}

pub fn send(stream: &mut UnixStream, value: &Value) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(Error::Invalid);
    }
    stream.write_all(
        &u32::try_from(bytes.len())
            .map_err(|_| Error::Invalid)?
            .to_be_bytes(),
    )?;
    stream.write_all(&bytes)?;
    Ok(())
}

pub fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, Error> {
    value[field]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or(Error::Invalid)
}
