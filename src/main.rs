mod app;
mod backend;
mod config;
mod connection;
mod error;
mod protocol;
mod secrets;

use std::time::Duration;

use gpui_kit::*;

use crate::app::AppState;
use crate::backend::authenticate;
use crate::connection::http::UnaryClient;
use crate::connection::ws;
use crate::error::AppError;

fn run_probe(input: &str) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (target, cookie) = authenticate(input).await?;
        tracing::info!(
            authority = %cookie.authority,
            name = %cookie.name,
            "authenticated with dsh"
        );
        let unary = UnaryClient::new(target.base.clone(), Some(cookie))?;
        let sessions = protocol::methods::session_list(&unary).await?;
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        Ok(())
    })
}

fn run_events(input: &str) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (target, cookie) = authenticate(input).await?;
        let ready = tokio::time::timeout(
            Duration::from_secs(20),
            ws::wait_events_ready(&target.base, &cookie, Duration::from_secs(15)),
        )
        .await
        .map_err(|_| AppError::Transport("events probe timed out".to_string()))??;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "clientId": ready.client_id,
                "home": ready.home,
            }))?
        );
        Ok(())
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dshg=info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--probe") => {
            let Some(url) = args.get(2) else {
                eprintln!("usage: dshg --probe <dsh-web-url>");
                std::process::exit(2);
            };
            if let Err(error) = run_probe(url) {
                eprintln!("error [{}]: {error}", error.code());
                std::process::exit(1);
            }
            return;
        }
        Some("--events") => {
            let Some(url) = args.get(2) else {
                eprintln!("usage: dshg --events <dsh-web-url>");
                std::process::exit(2);
            };
            if let Err(error) = run_events(url) {
                eprintln!("error [{}]: {error}", error.code());
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| AppState::new(window, cx))
        })
        .expect("failed to open window");
    });
}
