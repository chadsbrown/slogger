mod app;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("slogger starting");

    iced::application(app::App::init, app::App::update, app::App::view)
        .title(|_state: &app::App| "slogger".to_string())
        .subscription(app::App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1100.0, 800.0),
            ..Default::default()
        })
        .run()
        .map_err(|e| anyhow::anyhow!("iced exited: {e}"))
}
