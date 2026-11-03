use lab_runner::Evidence;
use lab_world::{Error, Result, World};

pub const SCENARIOS: [&str; 1] = ["RMT-PRIVATE-CONNECT"];

pub fn run(_world: &mut World, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.clone().as_str() {
        "RMT-PRIVATE-CONNECT" => crate::private::run(crate::replicas::binary()?, evidence),
        id => Err(Error::Invalid(format!("relay scenario not ported: {id}"))),
    }
}
