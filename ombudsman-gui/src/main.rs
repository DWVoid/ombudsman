//! ombudsman-gui: graphical chat client for ombudsman.
//!
//! Connects to `ombudsman-server` via WebSocket (msgpack protocol).
//! Built with iced using the MVVM pattern.

mod app;
mod view;
mod viewmodel;
mod ws_client;

fn main() -> iced::Result {
    // Initialize logging to stderr (doesn't interfere with the GUI).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("ombudsman_gui=debug".parse().unwrap())
                .add_directive("ombudsman_core=info".parse().unwrap()),
        )
        .with_target(false)
        .init();

    app::App::run()
}
