use super::{
    Arc, Frontdoor, HashMap, IntoResponse, Path, PathRejection, Principal, Query, QueryRejection,
    Response, State, StatusCode, extraction_problem, header, http_problem, problem,
};

pub(super) async fn blob(
    State(frontdoor): State<Arc<Frontdoor>>,
    digest: Result<Path<String>, PathRejection>,
    query: Result<Query<HashMap<String, String>>, QueryRejection>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
) -> Response {
    let digest = match digest {
        Ok(Path(digest)) => digest,
        Err(rejection) => return extraction_problem(rejection.status()),
    };
    let query = match query {
        Ok(Query(query)) => query,
        Err(rejection) => return extraction_problem(rejection.status()),
    };
    let Some(record) = query.get("record").filter(|record| !record.is_empty()) else {
        return http_problem(
            StatusCode::BAD_REQUEST,
            "invalid",
            "record is required",
            false,
        );
    };
    match blob_bytes(&frontdoor, &principal, record, &digest).await {
        Ok(bytes) => blob_response(bytes, &headers),
        Err(e) => problem(e),
    }
}

pub(crate) async fn blob_bytes(
    frontdoor: &Frontdoor,
    principal: &Principal,
    record: &str,
    digest: &str,
) -> Result<Vec<u8>, ab_operations::Error> {
    let current = frontdoor.service.identity.current(principal)?;
    if record.starts_with("qry_") {
        match &frontdoor.service.target {
            ab_operations::CommandTarget::Remote(target) => {
                target.query_blob(&current, record, digest).await
            }
            ab_operations::CommandTarget::Local(_) => {
                frontdoor
                    .service
                    .store
                    .cached_query_blob(&current, record, digest)
                    .await
            }
        }
    } else {
        frontdoor
            .service
            .store
            .blob(&current.visibility(), record, digest)
            .await
    }
}

fn blob_response(bytes: Vec<u8>, headers: &axum::http::HeaderMap) -> Response {
    let mut ranges = headers.get_all(header::RANGE).iter();
    if let Some(range) = ranges.next() {
        let total = bytes.len();
        if let Some((start, end)) = range
            .to_str()
            .ok()
            .filter(|_| ranges.next().is_none())
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
        let mut response = http_problem(
            StatusCode::RANGE_NOT_SATISFIABLE,
            "invalid_range",
            "invalid range",
            false,
        );
        if let Ok(value) = format!("bytes */{total}").parse() {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
        return response;
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
    if !start
        .bytes()
        .chain(end.bytes())
        .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
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
