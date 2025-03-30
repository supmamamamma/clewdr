use base64::{Engine, prelude::BASE64_STANDARD};
use futures::future::join_all;
use rquest::{
    Client, ClientBuilder, RequestBuilder,
    header::{COOKIE, ORIGIN, REFERER},
    multipart::{Form, Part},
};
use rquest_util::Emulation;
use serde_json::Value;
use std::sync::LazyLock;
use tracing::warn;

use crate::{types::message::ImageSource, utils::ENDPOINT};

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

pub async fn upload_images(
    imgs: Vec<ImageSource>,
    cookies: String,
    uuid_org: String,
) -> Vec<String> {
    // upload images
    let fut = imgs
        .into_iter()
        .map_while(|img| {
            if img.type_ != "base64" {
                warn!("Image type is not base64");
                return None;
            }
            let bytes = BASE64_STANDARD
                .decode(img.data.as_bytes())
                .inspect_err(|e| {
                    warn!("Failed to decode image: {:?}", e);
                })
                .ok()?;
            let file_name = match img.media_type.as_str() {
                "image/png" => "image.png",
                "image/jpeg" => "image.jpg",
                "image/gif" => "image.gif",
                "image/webp" => "image.webp",
                "application/pdf" => "document.pdf",
                _ => "file",
            };
            let part = Part::bytes(bytes).file_name(file_name);
            let form = Form::new().part("file", part);

            let endpoint = format!("https://claude.ai/api/{}/upload", uuid_org);
            Some(
                SUPER_CLIENT
                    .post(endpoint)
                    .append_headers("new", cookies.as_str())
                    .header_append("anthropic-client-platform", "web_claude_ai")
                    .multipart(form)
                    .send(),
            )
        })
        .collect::<Vec<_>>();

    // get upload responses
    let fut = join_all(fut)
        .await
        .into_iter()
        .map_while(|r| {
            r.inspect_err(|e| {
                warn!("Failed to upload image: {:?}", e);
            })
            .ok()
        })
        .map(|r| async {
            let json = r
                .json::<Value>()
                .await
                .inspect_err(|e| {
                    warn!("Failed to parse image response: {:?}", e);
                })
                .ok()?;
            Some(json["file_uuid"].as_str()?.to_string())
        })
        .collect::<Vec<_>>();

    join_all(fut)
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
}
