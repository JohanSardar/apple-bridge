use crate::Error;
use axum::body::Body;
use http_body::{Body as HttpBody, Frame, SizeHint};
use hyper::service::Service as _;
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::watch,
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadlines {
    pub(crate) first_request: Duration,
    pub(crate) between_requests: Duration,
    pub(crate) stalled_request: Duration,
    pub(crate) header_read: Duration,
}
impl Deadlines {
    pub(crate) const INGRESS: Self = Self {
        first_request: Duration::from_secs(15),
        between_requests: Duration::from_secs(60),
        stalled_request: Duration::from_mins(5),
        header_read: Duration::from_secs(10),
    };
}

#[derive(Debug)]
struct Activity {
    started: Instant,
    last_byte_millis: AtomicU64,
    requests_in_flight: AtomicUsize,
    any_request: AtomicBool,
}
impl Activity {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            last_byte_millis: AtomicU64::new(0),
            requests_in_flight: AtomicUsize::new(0),
            any_request: AtomicBool::new(false),
        })
    }
    fn elapsed_millis(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn touch(&self) {
        self.last_byte_millis
            .store(self.elapsed_millis(), Ordering::Relaxed);
    }
    fn expired(&self, deadlines: &Deadlines) -> bool {
        let quiet = Duration::from_millis(
            self.elapsed_millis()
                .saturating_sub(self.last_byte_millis.load(Ordering::Relaxed)),
        );
        if self.requests_in_flight.load(Ordering::Relaxed) > 0 {
            quiet >= deadlines.stalled_request
        } else if self.any_request.load(Ordering::Relaxed) {
            quiet >= deadlines.between_requests
        } else {
            self.started.elapsed() >= deadlines.first_request
        }
    }
    fn begin_request(self: &Arc<Self>) -> RequestInFlight {
        self.any_request.store(true, Ordering::Relaxed);
        self.requests_in_flight.fetch_add(1, Ordering::Relaxed);
        self.touch();
        RequestInFlight(self.clone())
    }
}

#[derive(Debug)]
struct RequestInFlight(Arc<Activity>);
impl Drop for RequestInFlight {
    fn drop(&mut self) {
        self.0.touch();
        self.0.requests_in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

struct TrackedBody {
    inner: Body,
    _request: RequestInFlight,
}
impl HttpBody for TrackedBody {
    type Data = <Body as HttpBody>::Data;
    type Error = <Body as HttpBody>::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

struct TrackedIo<S> {
    inner: S,
    activity: Arc<Activity>,
}
impl<S: AsyncRead + Unpin> AsyncRead for TrackedIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.activity.touch();
        }
        result
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for TrackedIo<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            self.activity.touch();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(crate) async fn serve<S, Svc>(
    socket: S,
    service: Svc,
    deadlines: Deadlines,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Svc: hyper::service::Service<
            hyper::Request<hyper::body::Incoming>,
            Response = hyper::Response<Body>,
            Error = std::convert::Infallible,
        > + Send
        + Sync
        + 'static,
    Svc::Future: Send + 'static,
{
    let activity = Activity::new();
    let tracked = {
        let activity = activity.clone();
        let service = Arc::new(service);
        hyper::service::service_fn(move |request| {
            let in_flight = activity.begin_request();
            let service = service.clone();
            async move {
                let response = service.call(request).await?;
                Ok::<_, std::convert::Infallible>(response.map(|inner| {
                    Body::new(TrackedBody {
                        inner,
                        _request: in_flight,
                    })
                }))
            }
        })
    };
    let io = TrackedIo {
        inner: socket,
        activity: activity.clone(),
    };
    let connection = hyper::server::conn::http1::Builder::new()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(deadlines.header_read)
        .serve_connection(hyper_util::rt::TokioIo::new(io), tracked)
        .with_upgrades();
    let tick = deadlines
        .first_request
        .min(deadlines.between_requests)
        .min(deadlines.stalled_request)
        / 4;
    let expired = async {
        while !activity.expired(&deadlines) {
            tokio::time::sleep(tick).await;
        }
    };
    tokio::select! {
        _ = stop.changed() => Ok(()),
        () = expired => Ok(()),
        result = connection => result.map_err(|_| Error::Invalid),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use futures_util::stream;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    const SHORT: Deadlines = Deadlines {
        first_request: Duration::from_millis(300),
        between_requests: Duration::from_millis(300),
        stalled_request: Duration::from_millis(900),
        header_read: Duration::from_millis(600),
    };

    fn answering() -> impl hyper::service::Service<
        hyper::Request<hyper::body::Incoming>,
        Response = hyper::Response<Body>,
        Error = std::convert::Infallible,
        Future = impl Send + 'static,
    > + Send
    + Sync
    + 'static {
        hyper::service::service_fn(
            |request: hyper::Request<hyper::body::Incoming>| async move {
                let body = match request.uri().path() {
                    "/stream" => Body::from_stream(stream::unfold(0, |n| async move {
                        if n == 6 {
                            return None;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Some((Ok::<_, std::io::Error>("tick\n"), n + 1))
                    })),
                    "/silent" => {
                        Body::from_stream(stream::pending::<Result<&str, std::io::Error>>())
                    }
                    _ => Body::from("ok"),
                };
                Ok::<_, std::convert::Infallible>(hyper::Response::new(body))
            },
        )
    }

    fn start() -> (
        DuplexStream,
        tokio::task::JoinHandle<Result<(), Error>>,
        watch::Sender<bool>,
    ) {
        let (client, server) = tokio::io::duplex(1 << 16);
        let (stop_tx, mut stop) = watch::channel(false);
        let serving =
            tokio::spawn(async move { serve(server, answering(), SHORT, &mut stop).await });
        (client, serving, stop_tx)
    }

    async fn finishes(
        serving: tokio::task::JoinHandle<Result<(), Error>>,
        within: Duration,
    ) -> bool {
        tokio::time::timeout(within, serving).await.is_ok()
    }

    async fn read_all_available(
        client: &mut DuplexStream,
        wanted: &[u8],
        within: Duration,
    ) -> Vec<u8> {
        let mut received = Vec::new();
        let mut buffer = [0; 4096];
        let _ = tokio::time::timeout(within, async {
            while !received.windows(wanted.len()).any(|part| part == wanted) {
                let n = client.read(&mut buffer).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..n]);
            }
        })
        .await;
        received
    }

    #[tokio::test]
    async fn a_connection_that_never_sends_a_request_is_closed() {
        let (_client, serving, _stop) = start();
        assert!(finishes(serving, Duration::from_secs(3)).await);
    }

    #[tokio::test]
    async fn a_connection_that_stops_between_requests_is_closed() {
        let (mut client, serving, _stop) = start();
        client
            .write_all(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        let reply = read_all_available(&mut client, b"ok", Duration::from_secs(2)).await;
        assert!(reply.windows(2).any(|part| part == b"ok"));
        assert!(finishes(serving, Duration::from_secs(3)).await);
    }

    #[tokio::test]
    async fn a_streaming_response_outlives_the_idle_deadlines() {
        let (mut client, serving, _stop) = start();
        client
            .write_all(b"GET /stream HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        let mut seen = 0;
        let mut buffer = [0; 4096];
        while seen < 6 {
            let n = tokio::time::timeout(Duration::from_secs(3), client.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert!(n > 0, "the stream was cut off after {seen} ticks");
            seen += String::from_utf8_lossy(&buffer[..n])
                .matches("tick")
                .count();
        }
        drop(client);
        assert!(finishes(serving, Duration::from_secs(3)).await);
    }

    #[tokio::test]
    async fn a_request_with_no_bytes_for_the_stall_deadline_is_closed() {
        let (mut client, serving, _stop) = start();
        client
            .write_all(b"GET /silent HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        assert!(finishes(serving, Duration::from_secs(4)).await);
        drop(client);
    }

    #[tokio::test]
    async fn half_a_request_header_does_not_hold_the_slot() {
        let (mut client, serving, _stop) = start();
        client.write_all(b"GET / HTTP/1.1\r\nhos").await.unwrap();
        assert!(finishes(serving, Duration::from_secs(3)).await);
    }
}
