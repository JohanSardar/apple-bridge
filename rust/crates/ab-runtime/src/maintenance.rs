use crate::{Error, HomeLock};
use std::path::Path;

pub fn restore(home: &Path, backup: &Path, accept_ledger_loss: bool) -> Result<(), Error> {
    let _lock = HomeLock::acquire(home)?;
    ab_state::restore(home, backup, accept_ledger_loss)?;
    Ok(())
}

pub fn backup(home: &Path, destination: &Path) -> Result<(), Error> {
    if destination.as_os_str().is_empty() || !home.join("state.db").is_file() {
        return Err(Error::Invalid);
    }
    let _lock = HomeLock::acquire(home)?;
    ab_state::State::<ab_state::Gateway>::open(home)?.backup(destination)?;
    Ok(())
}
