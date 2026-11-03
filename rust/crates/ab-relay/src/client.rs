use crate::{Error, Result};

pub async fn limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Denied)? {
        if chunk.len() > limit.saturating_sub(data.len()) {
            return Err(Error::Denied);
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
