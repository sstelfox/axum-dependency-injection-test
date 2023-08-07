use std::net::SocketAddr;

use axum::{Json, Router, Server};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::StatusCode;
use tracing::Level;
use tracing_subscriber::{EnvFilter, Layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[cfg(test)]
mod test_helpers;

#[derive(Clone, Debug)]
struct AppState {
}

#[tokio::main]
async fn main() {
    let (non_blocking_writer, _guard) = tracing_appender::non_blocking(std::io::stderr());
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();

    let stderr_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_writer(non_blocking_writer)
        .with_filter(env_filter);

    tracing_subscriber::registry().with(stderr_layer).init();

    let app_state = AppState {
    };

    run_server(app_state).await;
}

pub async fn basic_handler() -> Response {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

async fn run_server(app_state: AppState) {
    let router = Router::new()
        .route("/", get(basic_handler))
        .with_state(app_state);

    let service_stack = tower::ServiceBuilder::new();

    let addr: SocketAddr = "[::]:3000".parse().expect("the syntax to be valid");
    let app = service_stack.service(router);

    tracing::info!(addr = ?addr, "server listening");

    let _ = Server::bind(&addr)
        .serve(app.into_make_service())
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    use axum::Router;
    use axum::routing::get;
    use serde::Deserialize;

    #[tokio::test]
    async fn test_basic_handler() {
        let app = Router::new().route("/", get(basic_handler));

        let client = TestClient::new(app);
        let res = client.get("/").send().await;

        assert_eq!(res.status(), StatusCode::OK);

        #[derive(Deserialize)]
        struct Response {
            status: String,
        }

        let body: Response = res.json().await;
        assert_eq!(body.status.as_str(), "ok");
    }
}
