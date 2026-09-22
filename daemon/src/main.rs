mod health;
mod market;
mod order;
mod proof;
mod rpc;

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::cors::{AllowOrigin, CorsLayer};

use health::{
    connect_user, health_report, twine_info, Config, ConnectResult, Health, TwineInfo,
};
use market::{AdView, MarketStore};
use order::{
    BuyerInvoiceBody, CancelTradeBody, ConnectBody, CreateAdBody, CreateTradeBody, FiatSentBody,
    OpenDisputeBody, OrderError, PostChatBody, TradeView, HOLD_EXPIRY_POLL,
};
use rpc::FiberRpc;

#[derive(Clone)]
struct App {
    config: Config,
    rpc: FiberRpc,
    market: MarketStore,
}

#[derive(Debug, Deserialize)]
struct TradesQuery {
    pubkey: Option<String>,
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
    let market = match MarketStore::from_env() {
        Ok(market) => market,
        Err(err) => {
            eprintln!("market file: {err}");
            return ExitCode::from(1);
        }
    };
    let app_state = App {
        config: config.clone(),
        rpc: FiberRpc::new(),
        market: market.clone(),
    };
    tokio::spawn(hold_expiry_poller(app_state.clone()));
    let app = Router::new()
        .route("/health", get(health))
        .route("/twine", get(get_twine))
        .route("/connect", post(connect))
        .route("/ads", get(list_ads).post(create_ad))
        .route("/ads/{id}/cancel", post(cancel_ad))
        .route("/trades", get(list_trades).post(create_trade))
        .route("/trades/{id}", get(get_trade))
        .route("/trades/{id}/proof", get(get_proof))
        .route("/trades/{id}/demo_cancel", post(demo_cancel))
        .route("/trades/{id}/locked", post(mark_locked))
        .route("/trades/{id}/try_cancel", post(try_cancel))
        .route("/trades/{id}/cancel", post(cancel_trade))
        .route("/trades/{id}/fiat_sent", post(fiat_sent))
        .route("/trades/{id}/release", post(release))
        .route("/trades/{id}/retry", post(retry))
        .route("/trades/{id}/dispute", post(open_dispute))
        .route("/trades/{id}/chat", post(post_chat))
        .route("/trades/{id}/award_buyer", post(award_buyer))
        .route("/trades/{id}/award_seller", post(award_seller))
        .layer(local_cors())
        .with_state(app_state);
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

async fn hold_expiry_poller(app: App) {
    let mut ticker = tokio::time::interval(HOLD_EXPIRY_POLL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match app.market.poll_all_windows(&app.rpc, &app.config).await {
            Ok(changed) => {
                for view in changed {
                    eprintln!(
                        "window poll: trade={} state={:?} invoice={:?}",
                        view.id, view.state, view.invoice_status
                    );
                }
            }
            Err(err) => {
                eprintln!("window poll error: {err:?}");
            }
        }
        match app.market.poll_all_hold_expiry(&app.rpc, &app.config).await {
            Ok(expired) => {
                for view in expired {
                    eprintln!(
                        "path D poll: trade={} state={:?} invoice={:?}",
                        view.id, view.state, view.invoice_status
                    );
                }
            }
            Err(err) => {
                eprintln!("path D poll error: {err:?}");
            }
        }
    }
}

async fn health(State(app): State<App>) -> Json<Health> {
    Json(health_report(&app.rpc, &app.config).await)
}

async fn get_twine(State(app): State<App>) -> Json<TwineInfo> {
    Json(twine_info(&app.rpc, &app.config).await)
}

async fn connect(
    State(app): State<App>,
    Json(body): Json<ConnectBody>,
) -> Result<Json<ConnectResult>, ApiError> {
    connect_user(&app.rpc, &app.config, &body.pubkey, &body.address)
        .await
        .map(Json)
        .map_err(|message| ApiError::Order(OrderError::Fiber(message)))
}

async fn list_ads(State(app): State<App>) -> Json<Vec<AdView>> {
    Json(app.market.list_ads())
}

async fn create_ad(
    State(app): State<App>,
    Json(body): Json<CreateAdBody>,
) -> Result<Json<AdView>, ApiError> {
    app.market.create_ad(&body).map(Json).map_err(ApiError::from)
}

async fn cancel_ad(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<AdView>, ApiError> {
    app.market.cancel_ad(&id).map(Json).map_err(ApiError::from)
}

async fn list_trades(
    State(app): State<App>,
    Query(query): Query<TradesQuery>,
) -> Json<Vec<TradeView>> {
    Json(app.market.list_trades(query.pubkey.as_deref()))
}

async fn create_trade(
    State(app): State<App>,
    Json(body): Json<CreateTradeBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .create_trade(&app.rpc, &app.config, &body)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn get_trade(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market.get_trade(&id).map(Json).map_err(ApiError::from)
}

async fn demo_cancel(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .demo_cancel(&id, &app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn mark_locked(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .mark_locked(&id, &app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn try_cancel(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .try_cancel(&id, &app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn cancel_trade(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<CancelTradeBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .cancel_trade(&id, &app.rpc, &app.config, &body)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn get_proof(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let (content_type, bytes) = app.market.get_proof(&id).map_err(ApiError::from)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
            HeaderValue::from_static("application/octet-stream")
        }),
    );
    Ok((headers, bytes))
}

async fn fiat_sent(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<FiatSentBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .fiat_sent(&id, &body)
        .map(Json)
        .map_err(ApiError::from)
}

async fn release(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .release(&id, &app.rpc, &app.config)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn retry(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<BuyerInvoiceBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .retry(&id, &app.rpc, &app.config, &body)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn open_dispute(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<OpenDisputeBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .open_dispute(&id, &app.rpc, &app.config, &body)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn post_chat(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<PostChatBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .post_chat(&id, &body)
        .map(Json)
        .map_err(ApiError::from)
}

async fn award_buyer(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<BuyerInvoiceBody>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .award_buyer(&id, &app.rpc, &app.config, &body)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

async fn award_seller(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<TradeView>, ApiError> {
    app.market
        .award_seller(&id, &app.rpc, &app.config)
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
            OrderError::NotFound(message) => (StatusCode::NOT_FOUND, message),
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
