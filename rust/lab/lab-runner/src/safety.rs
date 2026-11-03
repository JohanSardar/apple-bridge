use crate::Evidence;
use lab_world::{Error, Result, World};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Owner {
        fence: u64,
    },
    Accepted {
        fence: u64,
    },
    Commit {
        checkpoint: String,
    },
    Ack {
        checkpoint: String,
    },
    Reopen {
        checkpoint: String,
    },
    Read {
        allowed: Vec<String>,
        sources: Vec<String>,
    },
}
pub fn observe(events: &[Event]) -> std::result::Result<(), &'static str> {
    let mut fence = 0;
    let mut commits = BTreeSet::new();
    let mut latest = None;
    for event in events {
        match event {
            Event::Owner { fence: next } => {
                if *next <= fence {
                    return Err("invalid-owner-sequence");
                }
                fence = *next;
            }
            Event::Accepted { fence: accepted } => {
                if fence == 0 || *accepted != fence {
                    return Err("accept-stale-fence");
                }
            }
            Event::Commit { checkpoint } => {
                if checkpoint.is_empty() || !commits.insert(checkpoint) {
                    return Err("invalid-commit-sequence");
                }
                latest = Some(checkpoint);
            }
            Event::Ack { checkpoint } => {
                if !commits.contains(checkpoint) {
                    return Err("ack-uncommitted-data");
                }
            }
            Event::Reopen { checkpoint } => {
                if latest != Some(checkpoint) {
                    return Err("skip-checkpoint-commit");
                }
            }
            Event::Read { allowed, sources } => {
                if sources.iter().any(|source| !allowed.contains(source)) {
                    return Err("leak-scope");
                }
            }
        }
    }
    Ok(())
}
pub fn gate(_: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let healthy: Vec<Event> = serde_json::from_value(input["healthy"].clone())?;
    let mutant: Vec<Event> = serde_json::from_value(input["mutant"].clone())?;
    let expected = input["fault"]
        .as_str()
        .ok_or_else(|| Error::Invalid("fault name required".into()))?;
    if ![
        "skip-checkpoint-commit",
        "accept-stale-fence",
        "ack-uncommitted-data",
        "leak-scope",
    ]
    .contains(&expected)
        || healthy.is_empty()
        || mutant.is_empty()
    {
        return Err(Error::Invalid("invalid fault gate input".into()));
    }
    evidence.check(
        "healthy_trace",
        observe(&healthy).is_ok(),
        "protocol observer control",
    );
    evidence.check(
        expected,
        observe(&mutant) == Err(expected),
        "seeded protocol trace rejected by named invariant; gateway mutation not exercised",
    );
    Ok(())
}
