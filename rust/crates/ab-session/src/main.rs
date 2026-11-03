#![forbid(unsafe_code)]

use ab_session::Error;

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 || args[0] != "--gateway-fd" || args[2] != "--helper-fd" {
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
