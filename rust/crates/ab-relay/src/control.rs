use ab_relay::server::{ConnectionClock, ConnectionDeadlines};
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tower::{Service, ServiceExt};
pub(super) async fn serve_control(
    control: Option<(TcpListener, Arc<Semaphore>)>,
    mut incoming: tokio::sync::mpsc::Receiver<ab_relay::server::ControlConnection>,
    mut make: IntoMakeServiceWithConnectInfo<axum::Router, SocketAddr>,
    acceptor: TlsAcceptor,
    stop_api: CancellationToken,
    deadlines: ConnectionDeadlines,
) {
    let mut workers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            () = stop_api.cancelled() => break,
            Some(_) = workers.join_next(), if !workers.is_empty() => {},
            accepted = accept_control(control.as_ref(), &mut incoming) => {
                let ab_relay::server::ControlConnection {
                    socket, peer, prefix, completed,
                } = match accepted {
                    Ok(Some(connection)) => connection,
                    Ok(None) => break,
                    Err(error) => {
                        eprintln!("control accept failed: {error}");
                        tokio::time::sleep(ab_relay::server::ACCEPT_RETRY).await;
                        continue;
                    }
                };
                let own_slot = match &control {
                    Some((_, slots)) if completed.is_none() => match slots.clone().try_acquire_owned() {
                        Ok(slot) => Some(slot),
                        Err(_) => continue,
                    },
                    _ => None,
                };
                let socket = ab_relay::replay::Replay::new(socket, prefix);
                let Ok(service) = make.call(peer).await;
                let acceptor = acceptor.clone();
                workers.spawn(serve_connection(
                    socket, service, acceptor, completed, own_slot, deadlines,
                ));
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
}

async fn accept_control(
    control: Option<&(TcpListener, Arc<Semaphore>)>,
    incoming: &mut tokio::sync::mpsc::Receiver<ab_relay::server::ControlConnection>,
) -> std::io::Result<Option<ab_relay::server::ControlConnection>> {
    tokio::select! {
        result = accept_listener(control) => result.map(Some),
        connection = incoming.recv() => Ok(connection),
    }
}
async fn accept_listener(
    control: Option<&(TcpListener, Arc<Semaphore>)>,
) -> std::io::Result<ab_relay::server::ControlConnection> {
    let (socket, peer) = match control {
        Some((listener, _)) => listener.accept().await?,
        None => std::future::pending().await,
    };
    Ok(ab_relay::server::ControlConnection {
        socket,
        peer,
        prefix: Vec::new(),
        completed: None,
    })
}
async fn serve_connection<ServiceType>(
    socket: ab_relay::replay::Replay,
    service: ServiceType,
    acceptor: TlsAcceptor,
    completed: Option<tokio::sync::oneshot::Sender<()>>,
    own_slot: Option<tokio::sync::OwnedSemaphorePermit>,
    deadlines: ConnectionDeadlines,
) where
    ServiceType: Service<
            hyper::Request<hyper::body::Incoming>,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    ServiceType::Future: Send,
{
    let (_completed, _own_slot) = (completed, own_slot);
    let clock = ConnectionClock::start(deadlines);
    let served = async {
        if let Ok(Ok(tls)) =
            tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)).await
        {
            let mut builder = Builder::new(TokioExecutor::new());
            builder
                .http1()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(5))
                .max_buf_size(12288);
            let counted = {
                let clock = clock.clone();
                tower::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let clock = clock.clone();
                    let service = service.clone();
                    async move {
                        clock.request_started();
                        let response = service.oneshot(request).await;
                        clock.request_finished();
                        response
                    }
                })
            };
            let _ = builder
                .serve_connection_with_upgrades(
                    TokioIo::new(tls),
                    TowerToHyperService::new(counted),
                )
                .await;
        }
    };
    tokio::select! {
        () = served => {},
        () = clock.expired() => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_routed_channel_ends_accept_without_a_listener() {
        let (sender, mut incoming) = tokio::sync::mpsc::channel(1);
        drop(sender);
        assert!(matches!(
            accept_control(None, &mut incoming).await,
            Ok(None)
        ));
    }

    #[tokio::test]
    async fn listener_connections_have_no_routing_permit_or_prefix() -> std::io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let client = tokio::net::TcpStream::connect(listener.local_addr()?).await?;
        let control = Some((listener, Arc::new(Semaphore::new(1))));
        let connection = accept_listener(control.as_ref()).await?;
        assert_eq!(connection.peer, client.local_addr()?);
        assert_eq!(connection.prefix, [] as [u8; 0]);
        assert!(connection.completed.is_none());
        Ok(())
    }
}
