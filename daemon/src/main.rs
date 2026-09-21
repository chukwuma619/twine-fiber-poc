mod health;
mod order;
mod rpc;

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::cors::{AllowOrigin, CorsLayer};

use health::{health_report, Config, Health};
use order::{CreateOrderBody, OrderError, OrderStore, OrderView};
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
        .route("/order/demo_cancel", post(demo_cancel))
        .route("/order/hold", post(create_hold))
        .route("/order/lock", post(lock_payment))
        .route("/order/try_cancel", post(try_cancel))
        .route("/order/accept", post(accept))
        .route("/order/fiat_sent", post(fiat_sent))
        .route("/order/release", post(release))
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

async fn get_order(State(app): State<App>) -> Json<OrderView> {
    Json(app.orders.snapshot())
}

async fn create_order(
    State(app): State<App>,
    Json(body): Json<CreateOrderBody>,
) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .create(&body.amount)
        .map(Json)
        .map_err(ApiError::from)
}

async fn demo_cancel(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .demo_cancel(&app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn create_hold(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .create_hold(&app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn lock_payment(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .lock_payment(&app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn try_cancel(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .try_cancel(&app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn accept(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders.accept().map(Json).map_err(ApiError::from)
}

async fn fiat_sent(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders.fiat_sent().map(Json).map_err(ApiError::from)
}

async fn release(State(app): State<App>) -> Result<Json<OrderView>, ApiError> {
    app.orders
        .release(&app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

enum ApiError {
    Order(OrderError),
}

impl From<OrderError> for ApiError {
    fn from(err: OrderError) -> Self {
        Self::Order(err)
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let ApiError::Order(err) = self;
        let (status, message) = match err {
            OrderError::BadAmount(message) | OrderError::BadState(message) => {
                (StatusCode::BAD_REQUEST, message)
            }
            OrderError::AlreadyOpen => {
                (StatusCode::CONFLICT, "an order is already open".to_string())
            }
            OrderError::Fiber(message) => (StatusCode::BAD_GATEWAY, message),
            OrderError::Save(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
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
