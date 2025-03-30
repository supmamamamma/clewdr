use clap::Parser;
use const_format::formatc;

pub mod bootstrap;
pub mod client;
pub mod config;
pub mod error;
pub mod messages;
pub mod router;
pub mod state;
pub mod utils;

pub const TITLE: &str = formatc!(
    "Clewdr v{} by {}",
    env!("CARGO_PKG_VERSION"),
    env!("CARGO_PKG_AUTHORS")
);

#[derive(Parser, Debug)]
pub struct Args {
    pub cookie_file: Option<String>,
}
