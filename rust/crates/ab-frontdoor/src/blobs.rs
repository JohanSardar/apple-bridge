use super::{
    Arc, Code, Frontdoor, HashMap, IntoResponse, Path, Principal, Query, Response, State,
    StatusCode, header, problem,
};

pub(super) async fn blob(
    State(frontdoor): State<Arc<Frontdoor>>,
    Path(digest): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(record) = query.get("record") else {
        return problem(Code::Unauthorized.into());
    };
    let current = match frontdoor.service.identity.current(&principal) {
        Ok(p) => p,
        Err(e) => return problem(e),
    };
    let result = if record.starts_with("qry_") {
        match &frontdoor.service.target {
            ab_operations::CommandTarget::Remote(target) => {
                target.query_blob(&current, record, &digest).await
            }
            ab_operations::CommandTarget::Local(_) => {
                frontdoor
                    .service
                    .store
                    .cached_query_blob(&current, record, &digest)
                    .await
            }
        }
    } else {
        frontdoor
            .service
            .store
            .blob(&current.visibility(), record, &digest)
            .await
    };
    match result {
        Ok(bytes) => blob_response(bytes, &headers),
        Err(e) => problem(e),
    }
}

fn blob_response(bytes: Vec<u8>, headers: &axum::http::HeaderMap) -> Response {
    if let Some(range) = headers.get(header::RANGE) {
        let total = bytes.len();
        if let Some((start, end)) = range
            .to_str()
            .ok()
            .and_then(|range| byte_range(range, total))
        {
            return (
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                    (header::ACCEPT_RANGES, "bytes".into()),
                    (
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{total}"),
                    ),
                ],
                bytes[start..=end].to_vec(),
            )
                .into_response();
        }
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            "invalid range\n",
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::ACCEPT_RANGES, "bytes"),
        ],
        bytes,
    )
        .into_response()
}
fn byte_range(raw: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let (start, end) = raw.strip_prefix("bytes=")?.trim().split_once('-')?;
    if start.is_empty() {
        let length = end.parse::<usize>().ok()?.min(total);
        return (length > 0).then_some((total - length, total - 1));
    }
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<usize>().ok()?.min(total - 1)
    };
    (start <= end && start < total).then_some((start, end))
}
