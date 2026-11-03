use crate::Error;
use std::{
    io::Cursor,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
#[derive(Debug)]
pub(crate) struct Prefixed<T> {
    prefix: Cursor<Vec<u8>>,
    stream: T,
}
impl<T: AsyncRead + Unpin> AsyncRead for Prefixed<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.prefix.position() < self.prefix.get_ref().len() as u64 {
            return Pin::new(&mut self.prefix).poll_read(cx, buf);
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for Prefixed<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
pub(crate) async fn request<T: AsyncRead + AsyncWrite + Unpin>(
    mut stream: T,
) -> Result<Option<Prefixed<T>>, Error> {
    let mut prefix = Vec::new();
    while !prefix.ends_with(b"\r\n\r\n") {
        if prefix.len() >= 16384 {
            return Err(Error::Invalid);
        }
        prefix.push(stream.read_u8().await?);
    }
    if !prefix.starts_with(b"GET /v1/feed ") {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        stream.shutdown().await?;
        return Ok(None);
    }
    Ok(Some(Prefixed {
        prefix: Cursor::new(prefix),
        stream,
    }))
}
