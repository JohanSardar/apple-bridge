use super::{
    BULK_CHUNK_BYTES, CALENDAR_LIVE_CHUNK_BYTES, Hub, LIVE_CHUNK_BYTES, OBSERVATION_LIMIT, Origin,
    name,
};
use crate::{
    Error,
    flight::{Flight, Refused, typed_rows},
};
use ab_wire::{
    messages::{IpcFrame, Observation},
    native::{Frame as Link, Scan, ScanFailed, Source},
};
use std::time::Duration;

fn limit(scan: &Scan) -> usize {
    match (scan.live, scan.source) {
        (true, Source::Calendar) => CALENDAR_LIVE_CHUNK_BYTES,
        (true, _) => LIVE_CHUNK_BYTES,
        (false, _) => BULK_CHUNK_BYTES,
    }
}

fn split(scan: &Scan) -> Result<Vec<Vec<Observation>>, Error> {
    let limit = if scan.whole { usize::MAX } else { limit(scan) };
    let ordered = scan
        .observations
        .iter()
        .filter(|row| row.kind != "absent")
        .chain(scan.observations.iter().filter(|row| row.kind == "absent"));
    let mut groups: Vec<Vec<Observation>> = vec![Vec::new()];
    let mut bytes = 0;
    for row in ordered {
        let size = serde_json::to_vec(row)?.len() + 1;
        if size >= OBSERVATION_LIMIT {
            return Err(Error::Invalid);
        }
        if bytes + size > limit && groups.last().is_some_and(|group| !group.is_empty()) {
            groups.push(Vec::new());
            bytes = 0;
        }
        if let Some(group) = groups.last_mut() {
            group.push(row.clone());
        }
        bytes += size;
    }
    Ok(groups)
}

enum Step {
    Sent,
    Closed,
}

fn send_chunk(
    hub: &Hub,
    scan: &Scan,
    source: &str,
    chunk: &[Observation],
    checkpoint: &str,
    last: bool,
) -> Result<Step, Error> {
    loop {
        let predecessor = {
            let mut core = hub.core()?;
            while core.flights.contains(source) {
                if core.closed {
                    return Ok(Step::Closed);
                }
                core = hub
                    .changed
                    .wait_timeout(core, Duration::from_millis(100))
                    .map_err(|_| Error::WorkerPanicked)?
                    .0;
            }
            core.flights.predecessor(source, &scan.generation)
        };
        let capture = scan
            .capture(chunk.to_vec(), predecessor.clone(), checkpoint.to_owned())
            .map_err(|_| Error::Invalid)?;
        let bytes = IpcFrame {
            t: "Capture".into(),
            capture: Some(capture),
            ..IpcFrame::default()
        }
        .encode()
        .map_err(|_| Error::Invalid)?;
        let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        let flight = Flight::new(
            scan.generation.clone(),
            checkpoint.to_owned(),
            size,
            typed_rows(chunk),
            Origin::Helper {
                scan: scan.id.clone(),
                last,
            },
        );
        let mut core = hub.core()?;
        if core.closed {
            return Ok(Step::Closed);
        }
        if core.flights.predecessor(source, &scan.generation) != predecessor {
            continue;
        }
        match core.flights.reserve(source, flight) {
            Ok(()) => {
                drop(core);
                hub.write_gateway(&bytes)?;
                return Ok(Step::Sent);
            }
            Err(Refused::TooLarge) => return Err(Error::Invalid),
            Err(Refused::Busy) => {}
            Err(Refused::Credit) => {
                drop(
                    hub.changed
                        .wait_timeout(core, Duration::from_millis(100))
                        .map_err(|_| Error::WorkerPanicked)?,
                );
            }
        }
    }
}

fn wait_free(hub: &Hub, source: &str) -> Result<bool, Error> {
    let mut core = hub.core()?;
    while core.flights.contains(source) {
        if core.closed {
            return Ok(false);
        }
        core = hub
            .changed
            .wait_timeout(core, Duration::from_millis(100))
            .map_err(|_| Error::WorkerPanicked)?
            .0;
    }
    Ok(true)
}

pub(super) fn run_scan(hub: &Hub, scan: &Scan) -> Result<(), Error> {
    let source = name(scan.source)?;
    let chunks = split(scan)?;
    let final_index = chunks.len() - 1;
    let checkpoint = hub.next_checkpoint();
    for (index, chunk) in chunks.iter().enumerate() {
        let last = index == final_index;
        let chunk_checkpoint = if last {
            checkpoint.clone()
        } else {
            format!("{checkpoint}-{index}")
        };
        if matches!(
            send_chunk(hub, scan, &source, chunk, &chunk_checkpoint, last)?,
            Step::Closed
        ) {
            return Ok(());
        }
        if !last && !wait_free(hub, &source)? {
            return Ok(());
        }
    }
    Ok(())
}

pub(super) fn scan_failed(hub: &Hub, id: &str, error: &Error) {
    let code = match error {
        Error::Invalid => "invalid",
        _ => "unavailable",
    };
    let _ = hub.send_link(&Link::ScanFailed(ScanFailed {
        id: id.to_owned(),
        code: code.into(),
    }));
}
