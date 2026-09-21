mod health;
mod order;
mod rpc;

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use tower_http::cors::{AllowOrigin, CorsLayer};

use health::{health_report, Config, Health};
use order::{CreateError, CreateOrderBody, Order, OrderStore};
use rpc::FiberRpc;

#[derive(Clone)]
struct App {
    config: Config,
    rpc: FiberRpc,
    orders: OrderStore,
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = Config::from_env();
    let listen: SocketAddr = match config.listen.parse() {
        Ok(listen) => listen,
        Err(err) => {
            eprintln!("invalid LISTEN {}: {err}", config.listen);
            return ExitCode::from(1);
        }
    };
    let orders = match OrderStore::from_env() {
        Ok(orders) => orders,
        Err(err) => {
            eprintln!("order file: {err}");
            return ExitCode::from(1);
        }
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/order", get(get_order).post(create_order))
        .layer(local_cors())
        .with_state(App {
            config,
            rpc: FiberRpc::new(),
            orders,
        });
    let listener = match tokio::net::TcpListener::bind(listen).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("bind {listen}: {err}");
            return ExitCode::from(1);
        }
    };
    eprintln!("twine daemon listening on http://{listen}");
    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("serve: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

async fn health(State(app): State<App>) -> Json<Health> {
    Json(health_report(&app.rpc, &app.config).await)
}

async fn get_order(State(app): State<App>) -> Json<Order> {
    Json(app.orders.snapshot())
}

async fn create_order(
    State(app): State<App>,
    Json(body): Json<CreateOrderBody>,
) -> Result<Json<Order>, ApiError> {
    app.orders
        .create(&body.amount)
        .map(Json)
        .map_err(ApiError::from)
}

enum ApiError {
    Create(CreateError),
}

impl From<CreateError> for ApiError {
    fn from(err: CreateError) -> Self {
        Self::Create(err)
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let ApiError::Create(err) = self;
        let (status, message) = match err {
            CreateError::BadAmount(message) => (StatusCode::BAD_REQUEST, message),
            CreateError::AlreadyOpen => {
                (StatusCode::CONFLICT, "an order is already open".to_string())
            }
            CreateError::Save(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

fn local_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _| {
            origin_is_local(origin)
        }))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(tower_http::cors::Any)
}

fn origin_is_local(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(origin) else {
        return false;
    };
    matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
}
