mod config;
mod connection;
mod error;
mod protocol;
mod secrets;

use std::time::Duration;

use gpui_kit::*;

use crate::connection::auth::{self, AuthTarget};
use crate::connection::http::UnaryClient;
use crate::connection::ws;
use crate::error::AppError;
use crate::secrets::SessionCookie;

struct Root;

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child("dshg · M0")
    }
}

fn http_client() -> Result<reqwest::Client, AppError> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?)
}

/// Parse the printed URL and exchange its token for a cookie.
async fn authenticate(input: &str) -> Result<(AuthTarget, SessionCookie), AppError> {
    let client = http_client()?;
    let target = auth::parse_auth_url(input)?;
    let cookie = auth::exchange_token(&client, &target).await?;
    Ok((target, cookie))
}

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
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Root))
            .expect("failed to open window");
    });
}
