//! Bounded response readers for calls to third-party identity providers.
//!
//! Timeouts alone do not cap memory: a fast, chunked endpoint can otherwise
//! make `Response::json` buffer an arbitrarily large payload.

use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum BoundedJsonError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("response exceeds the {limit}-byte limit")]
    TooLarge { limit: usize },
    #[error("invalid JSON response: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BoundedJsonError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BoundedJsonError::TooLarge { limit });
    }

    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(limit as u64) as usize);
    while let Some(chunk) = response.chunk().await? {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .filter(|length| *length <= limit)
            .ok_or(BoundedJsonError::TooLarge { limit })?;
        body.reserve(next_len.saturating_sub(body.capacity()));
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

pub async fn text(response: reqwest::Response, limit: usize) -> Result<String, BoundedJsonError> {
    let body = bytes(response, limit).await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

pub async fn json<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, BoundedJsonError> {
    let body = bytes(response, limit).await?;
    serde_json::from_slice(&body).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::BoundedJsonError;

    #[test]
    fn oversized_error_does_not_echo_response_data() {
        let error = BoundedJsonError::TooLarge { limit: 1024 };
        assert_eq!(error.to_string(), "response exceeds the 1024-byte limit");
    }
}
