//! The only place that performs HTTP.
//!
//! The web build uses `gloo-net`, so the browser handles CORS and TLS; native
//! builds use `reqwest`. Everything above this module builds URLs and bodies
//! and stays platform-agnostic.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
}

pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Perform a request. `body` is sent as JSON when present.
pub async fn request(
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<Response, String> {
    imp::request(method, url, headers, body).await
}

/// A short, service-tagged message for a non-2xx response.
pub fn error_message(service: &str, response: &Response) -> String {
    let body: String = response.body.chars().take(300).collect();
    format!("{service} returned {}: {body}", response.status)
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{Method, Response};
    use gloo_net::http::Request;

    pub async fn request(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, String> {
        let builder = match method {
            Method::Get => Request::get(url),
            Method::Post => Request::post(url),
            Method::Put => Request::put(url),
            Method::Patch => Request::patch(url),
        };
        let mut builder = builder;
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        let request = match body {
            Some(text) => builder
                .header("Content-Type", "application/json")
                .body(text.to_string())
                .map_err(|error| error.to_string())?,
            None => builder.build().map_err(|error| error.to_string())?,
        };
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        let body = response.text().await.map_err(|error| error.to_string())?;
        Ok(Response { status, body })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{Method, Response};

    pub async fn request(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, String> {
        let method = match method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
        };
        let client = reqwest::Client::new();
        let mut request = client.request(method, url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(text) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(text.to_string());
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| error.to_string())?;
        Ok(Response { status, body })
    }
}
