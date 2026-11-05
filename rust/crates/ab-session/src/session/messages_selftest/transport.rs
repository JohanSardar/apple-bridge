use super::{CALL_WAIT, Driven, HANDSHAKE_WAIT, run};
use crate::{
    Error,
    session::owned::{Probe, text},
};
use ab_wire::native::{self, Frame, Primitive, Session as NativeSession};
use serde_json::{Value, json, value::RawValue};
use std::{
    cell::{Cell, RefCell},
    io::Write,
    os::unix::net::UnixStream,
};

struct LinkProbe {
    stream: RefCell<UnixStream>,
    next: Cell<u64>,
}

impl LinkProbe {
    fn open(mut stream: UnixStream) -> Result<Self, Error> {
        stream.set_read_timeout(Some(HANDSHAKE_WAIT))?;
        stream.write_all(
            &Frame::Hello { owner_fence: 1 }
                .encode()
                .map_err(|_| Error::Invalid)?,
        )?;
        if !matches!(native::read_frame(&mut stream), Ok(Frame::Welcome)) {
            return Err(Error::Invalid);
        }
        stream.write_all(
            &Frame::Session(NativeSession::default())
                .encode()
                .map_err(|_| Error::Invalid)?,
        )?;
        Ok(Self {
            stream: RefCell::new(stream),
            next: Cell::new(0),
        })
    }
}

impl Probe for LinkProbe {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
        let unavailable = || "unavailable".to_owned();
        let id = {
            self.next.set(self.next.get() + 1);
            format!("selftest-{}", self.next.get())
        };
        let frame = Frame::Primitive(Primitive {
            id: id.clone(),
            op: operation.into(),
            args: Some(RawValue::from_string(args.to_string()).map_err(|_| unavailable())?),
        });
        let wait = CALL_WAIT;
        let mut stream = self.stream.borrow_mut();
        stream
            .set_read_timeout(Some(wait))
            .map_err(|_| unavailable())?;
        stream
            .write_all(&frame.encode().map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
        loop {
            match native::read_frame(&mut *stream).map_err(|_| unavailable())? {
                Frame::PrimitiveResult(answer) if answer.id == id => {
                    return serde_json::from_str(answer.result.get()).map_err(|_| unavailable());
                }
                Frame::PrimitiveError(failure) if failure.id == id => return Err(failure.code),
                Frame::Close => return Err(unavailable()),
                _ => {}
            }
        }
    }
}

pub(in crate::session) fn serve(link: UnixStream, request: &Value) -> Result<(), Error> {
    let probe = LinkProbe::open(link)?;
    let local = crate::session::messages_local::Local::over(&probe);
    run(&local, &Driven(&local), request)
        .map(|_| ())
        .map_err(|_| Error::Invalid)
}

pub(in crate::session) fn serve_cleanup(link: UnixStream, request: &Value) -> Result<(), Error> {
    let probe = LinkProbe::open(link)?;
    let local = crate::session::messages_local::Local::over(&probe);
    let report = local.cleanup_report(&probe, text(request, "tag"));
    let body = report.unwrap_or_else(|code| json!({"state": "unavailable", "reason": code}));
    std::fs::write(text(request, "report"), body.to_string())?;
    Ok(())
}
