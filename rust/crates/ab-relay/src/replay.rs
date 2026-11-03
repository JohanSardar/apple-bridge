use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

#[derive(Debug)]
pub struct Replay {
    socket: TcpStream,
    prefix: std::io::Cursor<Vec<u8>>,
}
impl Replay {
    pub fn new(socket: TcpStream, prefix: Vec<u8>) -> Self {
        Self {
            socket,
            prefix: std::io::Cursor::new(prefix),
        }
    }
}
impl AsyncRead for Replay {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let position = usize::try_from(self.prefix.position()).unwrap_or(usize::MAX);
        if position < self.prefix.get_ref().len() {
            let n = buf.remaining().min(self.prefix.get_ref().len() - position);
            buf.put_slice(&self.prefix.get_ref()[position..position + n]);
            self.prefix
                .set_position(u64::try_from(position + n).unwrap_or(u64::MAX));
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.socket).poll_read(cx, buf)
    }
}
impl AsyncWrite for Replay {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
