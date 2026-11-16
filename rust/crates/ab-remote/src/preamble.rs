use crate::{Error, Result};

pub(super) const INGRESS_MAGIC: &[u8] = b"ab-ingress/1\0";
pub(super) const INGRESS_HAS_SOURCE: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IngressPreamble {
    pub source: Option<std::net::IpAddr>,
}

pub async fn read_ingress_preamble<R>(socket: &mut R, proof: &[u8; 32]) -> Result<IngressPreamble>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use subtle::ConstantTimeEq;
    use tokio::io::AsyncReadExt;
    let mut magic = vec![0; INGRESS_MAGIC.len()];
    socket.read_exact(&mut magic).await?;
    let mut received = [0; 32];
    socket.read_exact(&mut received).await?;
    if magic != INGRESS_MAGIC || !bool::from(proof.ct_eq(&received)) {
        return Err(Error::Contract("ingress proof"));
    }
    let flags = socket.read_u8().await?;
    if flags & !INGRESS_HAS_SOURCE != 0 {
        return Err(Error::Contract("ingress flags"));
    }
    let mut preamble = IngressPreamble::default();
    if flags & INGRESS_HAS_SOURCE != 0 {
        let length = usize::from(socket.read_u8().await?);
        if length != 4 && length != 16 {
            return Err(Error::Contract("ingress source"));
        }
        let mut address = vec![0; length];
        socket.read_exact(&mut address).await?;
        preamble.source = <[u8; 4]>::try_from(address.as_slice())
            .map(std::net::IpAddr::from)
            .ok()
            .or_else(|| {
                <[u8; 16]>::try_from(address.as_slice())
                    .ok()
                    .map(std::net::IpAddr::from)
            });
    }
    Ok(preamble)
}
