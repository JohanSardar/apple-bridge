#![forbid(unsafe_code)]
use ab_relay_control::{BetterAuth, Control, RelayApi, router, sync_policy};
use std::{net::SocketAddr, path::Path, sync::Arc};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("ab-relay-control: {error}");
        std::process::exit(1);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let public_url = required("AB_PUBLIC_URL")?;
    let listen: SocketAddr = std::env::var("AB_RELAY_CONTROL_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8789".to_owned())
        .parse()?;
    if !listen.ip().is_loopback() {
        return Err(
            "the control service only listens on loopback; the relay terminates TLS".into(),
        );
    }
    let control = Arc::new(Control::open(
        Path::new(&required("AB_RELAY_CONTROL_DB")?),
        Path::new(&required("AB_RELAY_CONTROL_FENCE")?),
        &public_url,
    )?);
    let accounts_url =
        std::env::var("AB_ACCOUNTS_URL").unwrap_or_else(|_| "http://127.0.0.1:8790".to_owned());
    let accounts = BetterAuth::new(&accounts_url)?;
    let token = std::fs::read_to_string(required("AB_RELAY_ADMIN_TOKEN_FILE")?)?
        .trim()
        .to_owned();
    let connect = std::env::var("AB_RELAY_CONNECT")
        .ok()
        .map(|value| value.parse::<SocketAddr>())
        .transpose()?;
    let relay = Arc::new(RelayApi::new(&required("AB_RELAY_URL")?, token, connect)?);
    tokio::spawn(sync_policy(control.clone(), relay.clone()));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!("ab-relay-control listening on {listen}");
    axum::serve(
        listener,
        router(control, Arc::new(accounts), relay, &accounts_url)?
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
