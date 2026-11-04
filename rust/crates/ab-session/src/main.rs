#![forbid(unsafe_code)]

use ab_session::Error;
use std::os::fd::AsRawFd;

#[cfg(all(feature = "lab", feature = "rust-effects"))]
struct LabEffects {
    native: String,
    home: std::path::PathBuf,
    config: ab_session::effects::Config,
}

#[cfg(all(feature = "lab", feature = "rust-effects"))]
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

#[cfg(feature = "lab")]
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
            let options = ab_session::routed::Options {
                poll,
                watch,
                state,
                queries: ab_session::queries::NativeQueries::default(),
            };
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

fn inherited_descriptor(
    variable: &str,
    taken: &[i32],
) -> Result<std::os::unix::net::UnixStream, Error> {
    let descriptor = std::env::var(variable)
        .map_err(|_| Error::Invalid)?
        .parse::<i32>()
        .map_err(|_| Error::Invalid)?;
    if descriptor < 3 || taken.contains(&descriptor) {
        return Err(Error::Invalid);
    }
    Ok(ab_sys::inherited_unix_stream(descriptor)?)
}

const FACTS_DESCRIPTOR: &str = "AB_NATIVE_FACTS_FD";

type Links = (
    Option<ab_session::production::EffectsLink>,
    Option<std::os::unix::net::UnixStream>,
);

fn native_links(
    gateway: &std::os::unix::net::UnixStream,
    helper: &std::os::unix::net::UnixStream,
    effects_home: Option<String>,
    wants_facts: bool,
) -> Result<Links, Error> {
    let mut taken = vec![gateway.as_raw_fd(), helper.as_raw_fd()];
    let effects = match effects_home {
        Some(home) => {
            let native = inherited_descriptor("AB_NATIVE_FD", &taken)?;
            taken.push(native.as_raw_fd());
            Some(ab_session::production::EffectsLink {
                native,
                #[cfg(feature = "rust-effects")]
                routing: ab_session::production::effects_routing(std::path::Path::new(&home))?,
                home: std::path::PathBuf::from(home),
            })
        }
        None => None,
    };
    let facts = if wants_facts {
        let variable = if effects.is_none() && std::env::var_os(FACTS_DESCRIPTOR).is_none() {
            "AB_NATIVE_FD"
        } else {
            FACTS_DESCRIPTOR
        };
        Some(inherited_descriptor(variable, &taken)?)
    } else {
        None
    };
    Ok((effects, facts))
}

fn exit_with_parent() {
    let parent = std::os::unix::process::parent_id();
    if parent <= 1 {
        std::process::exit(0);
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if std::os::unix::process::parent_id() != parent {
                std::process::exit(0);
            }
        }
    });
}

#[derive(Default)]
struct Flags {
    effects_home: Option<String>,
    capture_home: Option<String>,
    session_secret: Option<String>,
    session_home: Option<String>,
    rust_commands: Option<String>,
}

fn parse_flags(args: &[String]) -> Result<Flags, Error> {
    let mut flags = Flags::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(Error::Invalid)?.clone();
        let slot = match flag.as_str() {
            "--effects-home" => &mut flags.effects_home,
            "--capture-home" => &mut flags.capture_home,
            "--session-secret" => &mut flags.session_secret,
            "--session-home" => &mut flags.session_home,
            "--rust-commands" => &mut flags.rust_commands,
            _ => return Err(Error::Invalid),
        };
        if slot.replace(value).is_some() {
            return Err(Error::Invalid);
        }
    }
    if flags.rust_commands.is_some() && flags.session_secret.is_none() {
        return Err(Error::Conflict);
    }
    Ok(flags)
}

#[cfg(feature = "lab")]
fn lab_session(args: &[String]) -> Result<(), Error> {
    let (args, rust_commands) = match args {
        [rest @ .., flag, sources] if flag == "--rust-commands" => (rest, Some(sources.clone())),
        all => (all, None),
    };
    let [_, listen, _, gateway, _, secret, _, home] = args else {
        return Err(Error::Invalid);
    };
    let options = ab_session::session::Options {
        rust_commands: rust_commands
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .filter(|source| !source.is_empty())
            .map(str::to_owned)
            .collect(),
    };
    let secret = std::fs::read(secret)?;
    let home = std::path::PathBuf::from(home);
    let capture = ab_session::production::load(&home)?;
    let _ = std::fs::remove_file(listen);
    let listener = std::os::unix::net::UnixListener::bind(listen)?;
    for link in listener.incoming() {
        let link = link?;
        let Ok(stream) = std::os::unix::net::UnixStream::connect(gateway) else {
            continue;
        };
        let (secret, home, options) = (secret.clone(), home.clone(), options.clone());
        let attached = capture
            .as_ref()
            .map(|loaded| loaded.attached(None))
            .transpose()?;
        std::thread::spawn(move || {
            let clock = std::sync::Arc::new(ab_session::session::SystemClock::start());
            if let Err(error) = ab_session::session::serve_attached(
                stream, link, &secret, &home, clock, &options, attached,
            ) {
                eprintln!("ab-session: {error}");
            }
        });
    }
    Ok(())
}

fn selftest_messages(args: &[String]) -> Result<(), Error> {
    let [descriptor, request] = args else {
        return Err(Error::Invalid);
    };
    let descriptor = descriptor.parse::<i32>().map_err(|_| Error::Invalid)?;
    let request: serde_json::Value = serde_json::from_str(request)?;
    exit_with_parent();
    let link = ab_sys::inherited_unix_stream(descriptor)?;
    ab_session::session::selftest_messages(link, &request)
}

fn session_capture(
    home: &str,
    gateway: &std::os::unix::net::UnixStream,
    helper: &std::os::unix::net::UnixStream,
) -> Result<Option<ab_session::session::Attached>, Error> {
    let Some(loaded) = ab_session::production::load(std::path::Path::new(home))? else {
        return Ok(None);
    };
    let facts = if loaded.wants_inherited_facts() {
        let taken = [gateway.as_raw_fd(), helper.as_raw_fd()];
        Some(inherited_descriptor(FACTS_DESCRIPTOR, &taken)?)
    } else {
        None
    };
    Ok(Some(loaded.attached(facts)?))
}

fn serve_session(
    gateway: i32,
    helper: i32,
    secret: &str,
    home: &str,
    rust_commands: Option<&str>,
) -> Result<(), Error> {
    keep_log(std::path::Path::new(home));
    let secret = ab_session::session::load_secret(std::path::Path::new(secret))?;
    let gateway = ab_sys::inherited_unix_stream(gateway)?;
    let helper = ab_sys::inherited_unix_stream(helper)?;
    let capture = session_capture(home, &gateway, &helper)?;
    let options = ab_session::session::Options {
        rust_commands: rust_commands
            .unwrap_or_default()
            .split(',')
            .filter(|source| !source.is_empty())
            .map(str::to_owned)
            .collect(),
    };
    ab_session::session::serve_attached(
        gateway,
        helper,
        &secret,
        std::path::Path::new(home),
        std::sync::Arc::new(ab_session::session::SystemClock::start()),
        &options,
        capture,
    )
}

#[cfg(feature = "lab")]
fn lab_entry(args: &[String]) -> Option<Result<(), Error>> {
    if (args.len() == 8 || args.len() == 10)
        && args[0] == "--lab-session-listen"
        && args[2] == "--lab-gateway"
        && args[4] == "--session-secret"
        && args[6] == "--session-home"
    {
        return Some(lab_session(args));
    }
    if args.len() == 4 && args[0] == "--lab-listen" && args[2] == "--lab-gateway" {
        return Some(lab_listen(&args[1], &args[3], None));
    }
    if args.len() == 6
        && args[0] == "--lab-listen"
        && args[2] == "--lab-gateway"
        && args[4] == "--lab-native"
    {
        return Some(lab_listen(&args[1], &args[3], Some(&args[5])));
    }
    None
}

#[allow(clippy::too_many_lines)]
const LOG_LIMIT: u64 = 4 << 20;

fn keep_log(home: &std::path::Path) {
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::create_dir_all(home.join("session"));
    let path = home.join("session").join("ab-session.log");
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
    else {
        return;
    };
    if ab_sys::redirect_stderr(&file).is_ok() {
        eprintln!(
            "ab-session: start pid {} parent {}",
            std::process::id(),
            std::os::unix::process::parent_id()
        );
    }
}

fn run() -> Result<(), Error> {
    ab_sys::reset_signals();
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "lab")]
    if let Some(result) = lab_entry(&args) {
        return result;
    }
    if args.first().map(String::as_str) == Some("--selftest-messages") {
        return selftest_messages(&args[1..]);
    }
    if args.len() < 4 || args[0] != "--gateway-fd" || args[2] != "--helper-fd" {
        return Err(Error::Invalid);
    }
    let gateway = args[1].parse::<i32>().map_err(|_| Error::Invalid)?;
    let helper = args[3].parse::<i32>().map_err(|_| Error::Invalid)?;
    exit_with_parent();
    if gateway < 3 || helper < 3 || gateway == helper {
        return Err(Error::Invalid);
    }
    let Flags {
        effects_home,
        capture_home,
        session_secret,
        session_home,
        rust_commands,
    } = parse_flags(&args[4..])?;

    if let Some(secret) = session_secret {
        if effects_home.is_some() || capture_home.is_some() {
            return Err(Error::Conflict);
        }
        let home = session_home.ok_or(Error::Invalid)?;
        return serve_session(gateway, helper, &secret, &home, rust_commands.as_deref());
    }
    if let Some(home) = &capture_home {
        keep_log(std::path::Path::new(home));
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
    let capture = match &capture_home {
        Some(home) => ab_session::production::load(std::path::Path::new(home))?,
        None => None,
    };
    let gateway = ab_sys::inherited_unix_stream(gateway)?;
    let helper = ab_sys::inherited_unix_stream(helper)?;
    let (effects, facts) = native_links(
        &gateway,
        &helper,
        effects_home,
        capture
            .as_ref()
            .is_some_and(ab_session::production::Loaded::wants_inherited_facts),
    )?;
    ab_session::production::serve_with_facts(gateway, helper, capture.as_ref(), effects, facts)
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => {
            eprintln!("ab-session: relay ended");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ab-session: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
