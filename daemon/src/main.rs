mod health;
mod rpc;

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use health::{health_report, Config, Health};
use rpc::FiberRpc;

#[derive(Clone)]
struct App {
    config: Config,
    rpc: FiberRpc,
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
    let app = Router::new().route("/health", get(health)).with_state(App {
        config,
        rpc: FiberRpc::new(),
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
