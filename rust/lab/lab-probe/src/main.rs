#![forbid(unsafe_code)]
use lab_probe::{Error, Input, Route, checks, serve_proxy};
use std::{
    io::Read,
    net::TcpListener,
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("remote") => {
            let mut raw = String::new();
            std::io::stdin().read_to_string(&mut raw)?;
            let input: Input = serde_json::from_str(&raw)?;
            let roots =
                rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            println!("{}", checks(&input, roots)?);
            Ok(())
        }
        Some("proxy") => {
            let route = Route::parse(
                &flag(&args, "--host").ok_or(Error::Invalid("route"))?,
                &flag(&args, "--ip").ok_or(Error::Invalid("route"))?,
                false,
            )?;
            let ready = PathBuf::from(flag(&args, "--ready").ok_or(Error::Invalid("route"))?);
            let listener = TcpListener::bind("127.0.0.1:0")?;
            serve_proxy(&listener, &route, &ready, &Arc::new(AtomicBool::new(false)))
        }
        _ => Err(Error::Invalid("probe action")),
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            println!("{}", serde_json::json!({"error": error.to_string()}));
            ExitCode::FAILURE
        }
    }
}
