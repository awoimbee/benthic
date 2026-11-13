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

/// A binary response, used by the git smart-HTTP transport.
pub struct RawResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Perform a request with a binary body and response. Headers, including the
/// content type, are passed through unchanged.
pub async fn request_raw(
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Result<RawResponse, String> {
    imp::request_raw(method, url, headers, body).await
}

/// A short, service-tagged message for a non-2xx response.
pub fn error_message(service: &str, response: &Response) -> String {
    let body: String = response.body.chars().take(300).collect();
    format!("{service} returned {}: {body}", response.status)
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{Method, RawResponse, Response};
    use gloo_net::http::{Request, RequestBuilder};

    fn builder(method: Method, url: &str, headers: &[(&str, &str)]) -> RequestBuilder {
        let mut builder = match method {
            Method::Get => Request::get(url),
            Method::Post => Request::post(url),
            Method::Put => Request::put(url),
            Method::Patch => Request::patch(url),
        };
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        builder
    }

    pub async fn request_raw(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<RawResponse, String> {
        let builder = builder(method, url, headers);
        let request = match body {
            Some(bytes) => builder
                .body(js_sys::Uint8Array::from(bytes))
                .map_err(|error| error.to_string())?,
            None => builder.build().map_err(|error| error.to_string())?,
        };
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        let body = response.binary().await.map_err(|error| error.to_string())?;
        Ok(RawResponse { status, body })
    }

    pub async fn request(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, String> {
        let builder = builder(method, url, headers);
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
    use super::{Method, RawResponse, Response};

    fn builder(method: Method, url: &str, headers: &[(&str, &str)]) -> reqwest::RequestBuilder {
        let method = match method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
        };
        let mut request = reqwest::Client::new().request(method, url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request
    }

    pub async fn request_raw(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<RawResponse, String> {
        let mut request = builder(method, url, headers);
        if let Some(bytes) = body {
            request = request.body(bytes.to_vec());
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|error| error.to_string())?
            .to_vec();
        Ok(RawResponse { status, body })
    }

    pub async fn request(
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Response, String> {
        let mut request = builder(method, url, headers);
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
