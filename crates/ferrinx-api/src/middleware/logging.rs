use axum::{body::Body, http::Request, middleware::Next, response::Response};
use tracing::{info, Instrument};
use uuid::Uuid;

pub async fn logging_middleware(req: Request<Body>, next: Next) -> Response {
    let request_id = Uuid::new_v4().to_string();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let start = std::time::Instant::now();

    info!(
        request_id = %request_id,
        method = %method,
        uri = %uri,
        "Request started"
    );

    let span = tracing::info_span!(
        "request",
        request_id = %request_id,
        method = %method,
        uri = %uri,
    );

    let response = next.run(req).instrument(span).await;

    let latency = start.elapsed();
    let status = response.status();

    info!(
        request_id = %request_id,
        method = %method,
        uri = %uri,
        status = %status.as_u16(),
        latency_ms = %latency.as_millis(),
        "Request completed"
    );

    response
}
