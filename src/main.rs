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

fn run_create(input: &str, cwd: Option<&str>) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (target, cookie) = authenticate(input).await?;
        let unary = UnaryClient::new(target.base.clone(), Some(cookie))?;
        let session_id = protocol::methods::create_session(&unary, cwd).await?;
        println!("{session_id}");
        Ok(())
    })
}

fn run_follow(input: &str, session_id: &str) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let (target, cookie) = authenticate(input).await?;
        let (tx, rx) = async_channel::unbounded();
        let session_id = session_id.to_string();
        let follow = tokio::spawn(async move {
            let _ = crate::connection::follow::follow_session(&target.base, &cookie, &session_id, tx).await;
        });
        for _ in 0..2 {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Ok(value)) => println!("{}", serde_json::to_string_pretty(&value)?),
                _ => break,
            }
        }
        follow.abort();
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
        Some("--create") => {
            let Some(url) = args.get(2) else {
                eprintln!("usage: dshg --create <dsh-web-url> [cwd]");
                std::process::exit(2);
            };
            let cwd = args.get(3).map(String::as_str);
            if let Err(error) = run_create(url, cwd) {
                eprintln!("error [{}]: {error}", error.code());
                std::process::exit(1);
            }
            return;
        }
        Some("--follow") => {
            let (Some(url), Some(session_id)) = (args.get(2), args.get(3)) else {
                eprintln!("usage: dshg --follow <dsh-web-url> <sessionId>");
                std::process::exit(2);
            };
            if let Err(error) = run_follow(url, session_id) {
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
        let (_, handle) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| AppState::new(window, cx))
        })
        .expect("failed to open window");

        if let Ok(url) = std::env::var("DSHG_URL") {
            let weak = handle.downgrade();
            cx.spawn(async move |cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let _ = weak.update(cx, |state, cx| state.start_connect(url, cx));
            })
            .detach();
        }
    });
}
