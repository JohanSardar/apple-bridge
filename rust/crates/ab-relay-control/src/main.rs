use ab_relay_control::{Control, Github, policy_router, router};
use std::{net::SocketAddr, path::Path, sync::Arc};

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("ab-relay-control: startup or service unavailable");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("AB_RELAY_CONTROL_ISOLATED").as_deref() != Ok("1") {
        return Err("isolated service flag required".into());
    }
    let database = std::env::var("AB_RELAY_CONTROL_DB")?;
    let fence = std::env::var("AB_RELAY_CONTROL_FENCE")?;
    let listen: SocketAddr = std::env::var("AB_RELAY_CONTROL_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8789".to_owned())
        .parse()?;
    if !listen.ip().is_loopback() {
        return Err("isolated loopback listener required".into());
    }
    let control = Arc::new(Control::open(Path::new(&database), Path::new(&fence))?);
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--create-invite") {
        let expires: i64 = args.get(2).ok_or("expiry required")?.parse()?;
        let uses: u32 = args.get(3).ok_or("use count required")?.parse()?;
        println!("{}", control.invite(expires, uses)?);
        return Ok(());
    }

    let provider = Arc::new(Github::new(
        std::env::var("AB_RELAY_GITHUB_CLIENT_ID")?,
        std::env::var("AB_RELAY_GITHUB_CLIENT_SECRET")?,
    )?);
    if let Ok(token) = std::env::var("AB_RELAY_CONTROL_POLICY_TOKEN") {
        let policy = policy_router(control.clone(), &token)?;
        let address: SocketAddr = std::env::var("AB_RELAY_CONTROL_POLICY_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:8790".to_owned())
            .parse()?;
        if !address.ip().is_loopback() {
            return Err("isolated policy loopback required".into());
        }
        let policy_listener = tokio::net::TcpListener::bind(address).await?;
        tokio::spawn(async move {
            if axum::serve(policy_listener, policy).await.is_err() {
                eprintln!("ab-relay-control: policy listener unavailable");
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(
        listener,
        router(control, provider).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
