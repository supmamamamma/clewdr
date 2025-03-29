use rquest::{
    Client, ClientBuilder, RequestBuilder,
    header::{COOKIE, ORIGIN, REFERER},
};
use rquest_util::Emulation;
use std::sync::LazyLock;

use crate::utils::ENDPOINT;

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

pub trait AppendHeaders {
    fn append_headers(self, refer: impl AsRef<str>, cookies: impl AsRef<str>) -> Self;
}

impl AppendHeaders for RequestBuilder {
    fn append_headers(self, refer: impl AsRef<str>, cookies: impl AsRef<str>) -> RequestBuilder {
        self.header_append(ORIGIN, ENDPOINT)
            .header_append(REFERER, header_ref(refer))
            .header_append(COOKIE, cookies.as_ref())
    }
}

fn header_ref<S: AsRef<str>>(ref_path: S) -> String {
    if ref_path.as_ref().is_empty() {
        format!("{}/", ENDPOINT)
    } else {
        format!("{}/chat/{}", ENDPOINT, ref_path.as_ref())
    }
}
