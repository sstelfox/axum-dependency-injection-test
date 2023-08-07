use std::net::SocketAddr;
use std::sync::Arc;

use axum::{async_trait, Json, Router, Server};
use axum::extract::{FromRef, FromRequestParts, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::StatusCode;
use serde::Serialize;
use tracing::Level;
use tracing_subscriber::{EnvFilter, Layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[cfg(test)]
mod test_helpers;

#[derive(Clone)]
pub struct AppState {
    data_repo: DynDataRepo,
}

#[async_trait]
trait DataRepo {
    async fn retrieve(&self, id: usize) -> Result<Data, DataRepoError>;
}

#[derive(Debug, Serialize)]
struct Data {
    id: usize,
}

enum DataRepoError {
    NotFound,
    InvalidRequest,
}

type DynDataRepo = Arc<dyn DataRepo + Send + Sync>;

impl axum::extract::FromRef<AppState> for DynDataRepo {
    fn from_ref(state: &AppState) -> Self {
        state.data_repo.clone()
    }
}

pub struct StateDataRepo(DynDataRepo);

#[async_trait]
impl<S> FromRequestParts<S> for StateDataRepo
where
    DynDataRepo: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(StateDataRepo(DynDataRepo::from_ref(state)))
    }
}

struct ProdDataRepo;

#[async_trait]
impl DataRepo for ProdDataRepo {
    async fn retrieve(&self, id: usize) -> Result<Data, DataRepoError> {
        if id >= 1_024 {
            Err(DataRepoError::InvalidRequest)
        } else if id > 10 {
            Err(DataRepoError::NotFound)
        } else {
            Ok(Data { id })
        }
    }
}

pub async fn basic_handler() -> Response {
    (StatusCode::OK, Json(serde_json::json!({"id": 100}))).into_response()
}

pub async fn data_state_handler(Path(id): Path<usize>, State(state): State<AppState>) -> Response {
    match state.data_repo.retrieve(id).await {
        Ok(data) => (StatusCode::OK, Json(data)).into_response(),
        Err(DataRepoError::InvalidRequest) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"status": "bad id"}))).into_response(),
        Err(DataRepoError::NotFound) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"status": "not found"}))).into_response(),
    }
}

pub async fn data_extract_handler(Path(id): Path<usize>, data_repo: StateDataRepo) -> Response {
    match data_repo.0.retrieve(id).await {
        Ok(data) => (StatusCode::OK, Json(data)).into_response(),
        _ => (StatusCode::IM_A_TEAPOT, Json(&serde_json::json!({"status": "teapot"}))).into_response(),
    }
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

    let data_repo = Arc::new(ProdDataRepo) as DynDataRepo;
    let app_state = AppState { data_repo };

    run_server(app_state).await;
}

async fn run_server(app_state: AppState) {
    let router = Router::new()
        .route("/", get(basic_handler))
        .route("/data/:id", get(data_state_handler))
        .route("/pot/:id", get(data_extract_handler))
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

    struct MockDataRepo(Result<Data, DataRepoError>);

    #[async_trait]
    impl DataRepo for MockDataRepo {
        async fn retrieve(&self, _id: usize) -> Result<Data, DataRepoError> {
            self.0.clone()
        }
    }

    // Our clone implementations don't need to be in the root crate..., this is just a silly demo
    // to find what is absolutely minimal to support this

    impl Clone for Data {
        fn clone(&self) -> Self {
            Self { id: self.id }
        }
    }

    impl Clone for DataRepoError {
        fn clone(&self) -> Self {
            match self {
                DataRepoError::NotFound => DataRepoError::NotFound,
                DataRepoError::InvalidRequest => DataRepoError::InvalidRequest,
            }
        }
    }

    #[derive(Deserialize)]
    struct Response {
        id: usize,
    }

    #[tokio::test]
    async fn test_basic_handler() {
        let app = Router::new().route("/", get(basic_handler));

        let client = TestClient::new(app);

        let res = client.get("/").send().await;
        assert_eq!(res.status(), StatusCode::OK);

        let body: Response = res.json().await;
        assert_eq!(body.id, 100);
    }

    #[tokio::test]
    async fn test_mocked_data_state_handler() {
        let app_state = AppState {
            data_repo: Arc::new(MockDataRepo(Ok(Data { id: 50 }))) as DynDataRepo,
        };

        let app = Router::new().route("/:id", get(data_state_handler)).with_state(app_state);

        let client = TestClient::new(app);

        let res = client.get("/50").send().await;
        assert_eq!(res.status(), StatusCode::OK);

        let body: Response = res.json().await;
        assert_eq!(body.id, 50);
    }

    struct FixedMock;

    #[async_trait]
    impl DataRepo for FixedMock {
        async fn retrieve(&self, _id: usize) -> Result<Data, DataRepoError> {
            Err(DataRepoError::NotFound)
        }
    }

    #[derive(Clone)]
    struct MockState;

    impl axum::extract::FromRef<MockState> for DynDataRepo {
        fn from_ref(_state: &MockState) -> Self {
            Arc::new(FixedMock)
        }
    }

    #[tokio::test]
    async fn test_mocked_extract_handler() {
        let app = Router::new().route("/:id", get(data_extract_handler)).with_state(MockState);

        let client = TestClient::new(app);

        let res = client.get("/50").send().await;
        assert_eq!(res.status(), StatusCode::IM_A_TEAPOT);
    }
}
