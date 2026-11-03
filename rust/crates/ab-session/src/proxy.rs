use crate::Error;
use std::{io, net::Shutdown, os::unix::net::UnixStream, thread};

pub fn relay(gateway: UnixStream, helper: UnixStream) -> Result<(), Error> {
    let mut gateway_read = gateway.try_clone()?;
    let mut helper_write = helper.try_clone()?;
    thread::scope(|scope| {
        let upstream = scope.spawn(|| {
            let result = io::copy(&mut gateway_read, &mut helper_write);
            if result.is_err() {
                let _ = gateway_read.shutdown(Shutdown::Both);
                let _ = helper_write.shutdown(Shutdown::Both);
            } else {
                let _ = helper_write.shutdown(Shutdown::Write);
            }
            result
        });
        let mut helper_read = helper;
        let mut gateway_write = gateway;
        let result = io::copy(&mut helper_read, &mut gateway_write);
        if result.is_err() {
            let _ = helper_read.shutdown(Shutdown::Both);
            let _ = gateway_write.shutdown(Shutdown::Both);
        } else {
            let _ = gateway_write.shutdown(Shutdown::Write);
        }
        let upstream_result = upstream.join().map_err(|_| Error::WorkerPanicked)?;
        result?;
        upstream_result?;
        Ok(())
    })
}
