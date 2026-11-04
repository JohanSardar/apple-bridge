#![forbid(unsafe_code)]

use ab_session::Error;
#[cfg(feature = "rust-capture")]
use std::os::fd::AsRawFd;

#[cfg(all(feature = "rust-capture", feature = "rust-effects"))]
struct LabEffects {
    native: String,
    home: std::path::PathBuf,
    config: ab_session::effects::Config,
}

#[cfg(all(feature = "rust-capture", feature = "rust-effects"))]
fn lab_effects(
    native: Option<&str>,
    rust: &[String],
    config: &str,
) -> Result<Option<LabEffects>, Error> {
    let Some(native) = native else {
        return Ok(None);
    };
    let home = std::env::var("AB_SESSION_EFFECTS_HOME").map_err(|_| Error::Invalid)?;
    let routing = std::env::var("AB_SESSION_EFFECTS_SOURCES").map_or_else(
        |_| ab_session::effects::Routing::all(),
        |list| ab_session::effects::Routing::parse(&list),
    );
    let sources: serde_json::Value = serde_json::from_str(config)?;
    let notes_database = sources["notes"]["root"]
        .as_str()
        .map(|root| std::path::Path::new(root).join("NoteStore.sqlite"));
    Ok(Some(LabEffects {
        native: native.to_owned(),
        home: std::path::PathBuf::from(home),
        config: ab_session::effects::Config {
            routing,
            rust_sources: rust.iter().cloned().collect(),
            notes_database,
            ..ab_session::effects::Config::default()
        },
    }))
}

#[cfg(feature = "rust-capture")]
fn lab_listen(listen: &str, gateway: &str, native: Option<&str>) -> Result<(), Error> {
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
    selection.validate_routed(ab_session::production::SUPPORTED)?;
    let config = std::env::var("AB_SESSION_RUST_SOURCES").map_err(|_| Error::Invalid)?;
    let poll = std::time::Duration::from_millis(
        std::env::var("AB_SESSION_RUST_POLL_MS")
            .ok()
            .map_or(Ok(100), |v| v.parse::<u64>())
            .map_err(|_| Error::Invalid)?,
    );
    let state = std::env::var("AB_SESSION_RUST_STATE")
        .ok()
        .map(std::path::PathBuf::from);
    let watch = std::env::var("AB_SESSION_RUST_WATCH").as_deref() == Ok("1");
    #[cfg(feature = "rust-effects")]
    let effects = lab_effects(native, &rust, &config)?;
    #[cfg(not(feature = "rust-effects"))]
    if native.is_some() {
        return Err(Error::Conflict);
    }
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
        let Ok(gateway) = std::os::unix::net::UnixStream::connect(gateway) else {
            continue;
        };
        let readers = build()?;
        let state = state.clone();
        #[cfg(feature = "rust-effects")]
        let layer = match &effects {
            Some(effects) => {
                let Ok(stream) = std::os::unix::net::UnixStream::connect(&effects.native) else {
                    continue;
                };
                Some(ab_session::effects::Layer::open_with(
                    stream,
                    &effects.home,
                    effects.config.clone(),
                )?)
            }
            None => None,
        };
        std::thread::spawn(move || {
            let options = ab_session::routed::Options { poll, watch, state };
            #[cfg(feature = "rust-effects")]
            let overlay: Option<&dyn ab_session::routed::Overlay> = layer
                .as_ref()
                .map(|layer| layer as &dyn ab_session::routed::Overlay);
            #[cfg(not(feature = "rust-effects"))]
            let overlay: Option<&dyn ab_session::routed::Overlay> = None;
            if let Err(error) =
                ab_session::routed::relay_layered(gateway, helper, readers, &options, overlay)
            {
                eprintln!("ab-session: {error}");
            }
        });
    }
    Ok(())
}

#[cfg(feature = "rust-capture")]
fn inherited_native(
    gateway: &std::os::unix::net::UnixStream,
    helper: &std::os::unix::net::UnixStream,
) -> Result<std::os::unix::net::UnixStream, Error> {
    let native = std::env::var("AB_NATIVE_FD")
        .map_err(|_| Error::Invalid)?
        .parse::<i32>()
        .map_err(|_| Error::Invalid)?;
    if native < 3 || native == gateway.as_raw_fd() || native == helper.as_raw_fd() {
        return Err(Error::Invalid);
    }
    Ok(ab_sys::inherited_unix_stream(native)?)
}

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "rust-capture")]
    if args.len() == 4 && args[0] == "--lab-listen" && args[2] == "--lab-gateway" {
        return lab_listen(&args[1], &args[3], None);
    }
    #[cfg(feature = "rust-capture")]
    if args.len() == 6
        && args[0] == "--lab-listen"
        && args[2] == "--lab-gateway"
        && args[4] == "--lab-native"
    {
        return lab_listen(&args[1], &args[3], Some(&args[5]));
    }
    if args.len() < 4 || args[0] != "--gateway-fd" || args[2] != "--helper-fd" {
        return Err(Error::Invalid);
    }
    let gateway = args[1].parse::<i32>().map_err(|_| Error::Invalid)?;
    let helper = args[3].parse::<i32>().map_err(|_| Error::Invalid)?;
    if gateway < 3 || helper < 3 || gateway == helper {
        return Err(Error::Invalid);
    }
    let mut effects_home = None;
    let mut capture_home = None;
    let mut rest = args[4..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(Error::Invalid)?;
        match flag.as_str() {
            "--effects-home" if effects_home.is_none() => effects_home = Some(value.clone()),
            "--capture-home" if capture_home.is_none() => capture_home = Some(value.clone()),
            _ => return Err(Error::Invalid),
        }
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
    let effects_selected = std::env::var("AB_SESSION_EFFECTS").as_deref() == Ok("1");
    if effects_selected && effects_home.is_none() {
        return Err(Error::Conflict);
    }
    #[cfg(feature = "rust-capture")]
    let capture = match &capture_home {
        Some(home) => ab_session::production::load(std::path::Path::new(home))?,
        None => None,
    };
    #[cfg(not(feature = "rust-capture"))]
    if capture_home.is_some() {
        return Err(Error::Conflict);
    }
    let gateway = ab_sys::inherited_unix_stream(gateway)?;
    let helper = ab_sys::inherited_unix_stream(helper)?;
    #[cfg(feature = "rust-capture")]
    #[allow(clippy::needless_return)]
    {
        let effects = match effects_home {
            Some(home) => {
                let native = std::env::var("AB_NATIVE_FD")
                    .map_err(|_| Error::Invalid)?
                    .parse::<i32>()
                    .map_err(|_| Error::Invalid)?;
                if native < 3 || native == gateway.as_raw_fd() || native == helper.as_raw_fd() {
                    return Err(Error::Invalid);
                }
                Some(ab_session::production::EffectsLink {
                    native: ab_sys::inherited_unix_stream(native)?,
                    routing: ab_session::production::effects_routing(std::path::Path::new(&home))?,
                    home: std::path::PathBuf::from(home),
                })
            }
            None => None,
        };
        let facts = match &capture {
            Some(loaded) if loaded.wants_inherited_facts() => {
                if effects.is_some() {
                    return Err(Error::Conflict);
                }
                Some(inherited_native(&gateway, &helper)?)
            }
            _ => None,
        };
        return ab_session::production::serve_with_facts(
            gateway,
            helper,
            capture.as_ref(),
            effects,
            facts,
        );
    }
    #[cfg(not(feature = "rust-capture"))]
    {
        if effects_home.is_some() {
            return Err(Error::Conflict);
        }
        ab_session::proxy::relay(gateway, helper)
    }
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
