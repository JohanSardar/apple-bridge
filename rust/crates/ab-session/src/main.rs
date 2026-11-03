#![forbid(unsafe_code)]

use ab_session::Error;
#[cfg(feature = "rust-effects")]
use std::os::fd::AsRawFd;

#[cfg(feature = "rust-capture")]
fn lab_listen(listen: &str, gateway: &str) -> Result<(), Error> {
    let mut selection = ab_session::ownership::Selection::default();
    if let Ok(value) = std::env::var("AB_SESSION_PRODUCERS") {
        let sources: std::collections::BTreeMap<
            ab_session::ownership::SourceId,
            ab_session::ownership::Producer,
        > = serde_json::from_str(&value)?;
        for (source, producer) in sources {
            selection.select_disconnected(
                source,
                producer,
                &ab_session::ownership::ConnectionState::Disconnected,
            )?;
        }
    }
    let rust: Vec<String> = selection
        .rust_sources()
        .iter()
        .map(|source| {
            serde_json::to_value(source).map(|v| v.as_str().unwrap_or_default().to_owned())
        })
        .collect::<Result<_, _>>()?;
    selection.validate_routed(&[
        "safari",
        "usage",
        "notes",
        "phone",
        "voicemail",
        "voice_memos",
    ])?;
    let config = std::env::var("AB_SESSION_RUST_SOURCES").map_err(|_| Error::Invalid)?;
    let poll = std::time::Duration::from_millis(
        std::env::var("AB_SESSION_RUST_POLL_MS")
            .ok()
            .map_or(Ok(100), |v| v.parse::<u64>())
            .map_err(|_| Error::Invalid)?,
    );
    let build = move || -> Result<_, Error> {
        let mut readers = ab_session::readers::from_config(&config)?;
        readers.retain(|source, _| rust.contains(source));
        if readers.len() != rust.len() {
            return Err(Error::Invalid);
        }
        Ok(readers)
    };
    build()?;
    let _ = std::fs::remove_file(listen);
    let listener = std::os::unix::net::UnixListener::bind(listen)?;
    for helper in listener.incoming() {
        let helper = helper?;
        let gateway = std::os::unix::net::UnixStream::connect(gateway)?;
        let readers = build()?;
        std::thread::spawn(move || {
            if let Err(error) = ab_session::routed::relay(gateway, helper, readers, poll) {
                eprintln!("ab-session: {error}");
            }
        });
    }
    Ok(())
}

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "rust-capture")]
    if args.len() == 4 && args[0] == "--lab-listen" && args[2] == "--lab-gateway" {
        return lab_listen(&args[1], &args[3]);
    }
    if ![4, 6].contains(&args.len()) || args[0] != "--gateway-fd" || args[2] != "--helper-fd" {
        return Err(Error::Invalid);
    }
    let gateway = args[1].parse::<i32>().map_err(|_| Error::Invalid)?;
    let helper = args[3].parse::<i32>().map_err(|_| Error::Invalid)?;
    if gateway < 3 || helper < 3 || gateway == helper {
        return Err(Error::Invalid);
    }
    let mut selection = ab_session::ownership::Selection::default();
    if let Ok(value) = std::env::var("AB_SESSION_PRODUCERS") {
        let sources: std::collections::BTreeMap<
            ab_session::ownership::SourceId,
            ab_session::ownership::Producer,
        > = serde_json::from_str(&value)?;
        for (source, producer) in sources {
            selection.select_disconnected(
                source,
                producer,
                &ab_session::ownership::ConnectionState::Disconnected,
            )?;
        }
    }
    selection.validate_proxy()?;
    let gateway = ab_sys::inherited_unix_stream(gateway)?;
    let helper = ab_sys::inherited_unix_stream(helper)?;
    if args.len() == 6 {
        if args[4] != "--effects-home" || std::env::var("AB_SESSION_EFFECTS").as_deref() != Ok("1")
        {
            return Err(Error::Invalid);
        }
        #[cfg(feature = "rust-effects")]
        {
            let native = std::env::var("AB_NATIVE_FD")
                .map_err(|_| Error::Invalid)?
                .parse::<i32>()
                .map_err(|_| Error::Invalid)?;
            if native < 3 || native == gateway.as_raw_fd() || native == helper.as_raw_fd() {
                return Err(Error::Invalid);
            }
            return ab_session::effects::relay(
                gateway,
                helper,
                ab_sys::inherited_unix_stream(native)?,
                std::path::Path::new(&args[5]),
            );
        }
        #[cfg(not(feature = "rust-effects"))]
        return Err(Error::Conflict);
    }
    if std::env::var("AB_SESSION_EFFECTS").as_deref() == Ok("1") {
        return Err(Error::Conflict);
    }
    ab_session::proxy::relay(gateway, helper)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ab-session: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
