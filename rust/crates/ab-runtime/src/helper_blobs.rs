use ab_operations::Code;

#[derive(Debug)]
pub struct HelperBlobs(pub ab_applepeer::Commands);
impl ab_blobs::BlobRead for HelperBlobs {
    async fn read(
        &self,
        handle: &str,
        offset: u64,
        length: usize,
    ) -> Result<(Vec<u8>, bool), ab_blobs::Error> {
        self.0
            .read_blob(
                handle,
                i64::try_from(offset).map_err(|_| ab_blobs::Error::Invalid)?,
                i64::try_from(length).map_err(|_| ab_blobs::Error::Invalid)?,
            )
            .await
            .map_err(|error| match error.code {
                Code::Corrupt => ab_blobs::Error::Corrupt,
                Code::Invalid => ab_blobs::Error::Invalid,
                code => ab_blobs::Error::Io(std::io::Error::new(
                    match code {
                        Code::NotFound => std::io::ErrorKind::NotFound,
                        Code::Unsupported => std::io::ErrorKind::Unsupported,
                        Code::Conflict => std::io::ErrorKind::WouldBlock,
                        _ => std::io::ErrorKind::NotConnected,
                    },
                    "Helper blob read failed",
                )),
            })
    }
}
