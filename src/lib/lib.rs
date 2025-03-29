use clap::Parser;
use const_format::formatc;

pub mod router;
pub mod completion;
pub mod config;
pub mod error;
pub mod stream;
pub mod text;
pub mod utils;
pub mod bootstrap;
pub mod state;
pub mod client;

pub const TITLE: &str = formatc!(
    "Clewdr v{} by {}",
    env!("CARGO_PKG_VERSION"),
    env!("CARGO_PKG_AUTHORS")
);

#[derive(Parser, Debug)]
pub struct Args {
    pub cookie_file: Option<String>,
}