#![forbid(unsafe_code)]

use ab_session::Error;
use std::os::fd::AsRawFd;

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
            "--session-secret" => &mut flags.session_secret,
            "--session-home" => &mut flags.session_home,
            "--rust-commands" => &mut flags.rust_commands,
            _ => return Err(Error::Invalid),
        };
        if slot.replace(value).is_some() {
            return Err(Error::Invalid);
        }
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

fn cleanup_messages(args: &[String]) -> Result<(), Error> {
    let [descriptor, request] = args else {
        return Err(Error::Invalid);
    };
    let descriptor = descriptor.parse::<i32>().map_err(|_| Error::Invalid)?;
    let request: serde_json::Value = serde_json::from_str(request)?;
    exit_with_parent();
    let link = ab_sys::inherited_unix_stream(descriptor)?;
    ab_session::session::cleanup_messages(link, &request)
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

const LOG_LIMIT: u64 = 4 << 20;
const OPEN_FILES: u64 = 4096;

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
    ab_sys::raise_open_files(OPEN_FILES);
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(feature = "lab")]
    if (args.len() == 8 || args.len() == 10)
        && args[0] == "--lab-session-listen"
        && args[2] == "--lab-gateway"
        && args[4] == "--session-secret"
        && args[6] == "--session-home"
    {
        return lab_session(&args);
    }
    if let ["--media-cleanup-check", path] = args.iter().map(String::as_str).collect::<Vec<_>>()[..]
    {
        let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let answer =
            ab_session::messages_policy::media_cleanup_check(&fixture).ok_or(Error::Invalid)?;
        println!("{answer}");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--selftest-messages") {
        return selftest_messages(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("--cleanup-messages") {
        return cleanup_messages(&args[1..]);
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
        session_secret,
        session_home,
        rust_commands,
    } = parse_flags(&args[4..])?;
    let secret = session_secret.ok_or(Error::Invalid)?;
    let home = session_home.ok_or(Error::Invalid)?;
    serve_session(gateway, helper, &secret, &home, rust_commands.as_deref())
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
