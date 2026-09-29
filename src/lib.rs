pub mod agent;
pub mod config;
mod control_ui;
pub mod hub;
mod hub_storage;
mod hub_ui;
pub mod model;
pub mod scan;
pub mod store;
pub mod transfer;
pub mod tui;

pub fn init_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
