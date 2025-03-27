use const_format::formatc;
use rquest::{Client, ClientBuilder};
use rquest_util::Emulation;
use std::sync::LazyLock;

pub mod api;
pub mod completion;
pub mod config;
pub mod stream;
pub mod utils;
pub mod text;

pub static NORMAL_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    ClientBuilder::new()
        .build()
        .expect("Failed to create client")
});

pub static SUPER_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    ClientBuilder::new()
        .emulation(Emulation::Chrome134)
        .build()
        .expect("Failed to create client")
});

pub const TITLE: &str = formatc!(
    "Clewdr v{} by {}",
    env!("CARGO_PKG_VERSION"),
    env!("CARGO_PKG_AUTHORS")
);