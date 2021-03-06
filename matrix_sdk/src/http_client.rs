// Copyright 2020 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{convert::TryFrom, fmt::Debug, sync::Arc};

use http::{HeaderValue, Method as HttpMethod, Response as HttpResponse};
use reqwest::{Client, Response};
use tracing::trace;
use url::Url;

use matrix_sdk_common::{
    api::r0::media::create_content, async_trait, locks::RwLock, AsyncTraitDeps, AuthScheme,
    FromHttpResponseError,
};

use crate::{ClientConfig, Error, OutgoingRequest, Result, Session};

/// Abstraction around the http layer. The allows implementors to use different
/// http libraries.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait HttpSend: AsyncTraitDeps {
    /// The method abstracting sending request types and receiving response types.
    ///
    /// This is called by the client every time it wants to send anything to a homeserver.
    ///
    /// # Arguments
    ///
    /// * `request` - The http request that has been converted from a ruma `Request`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::convert::TryFrom;
    /// use matrix_sdk::{HttpSend, Result, async_trait};
    ///
    /// #[derive(Debug)]
    /// struct Client(reqwest::Client);
    ///
    /// impl Client {
    ///     async fn response_to_http_response(
    ///         &self,
    ///         mut response: reqwest::Response,
    ///     ) -> Result<http::Response<Vec<u8>>> {
    ///         // Convert the reqwest response to a http one.
    ///         todo!()
    ///     }
    /// }
    ///
    /// #[async_trait]
    /// impl HttpSend for Client {
    ///     async fn send_request(&self, request: http::Request<Vec<u8>>) -> Result<http::Response<Vec<u8>>> {
    ///         Ok(self
    ///             .response_to_http_response(
    ///                 self.0
    ///                     .execute(reqwest::Request::try_from(request)?)
    ///                     .await?,
    ///             )
    ///             .await?)
    ///     }
    /// }
    /// ```
    async fn send_request(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>>;
}

#[derive(Clone, Debug)]
pub(crate) struct HttpClient {
    pub(crate) inner: Arc<dyn HttpSend>,
    pub(crate) homeserver: Arc<Url>,
    pub(crate) session: Arc<RwLock<Option<Session>>>,
}

impl HttpClient {
    async fn send_request<Request: OutgoingRequest>(
        &self,
        request: Request,
        session: Arc<RwLock<Option<Session>>>,
        content_type: Option<HeaderValue>,
    ) -> Result<http::Response<Vec<u8>>> {
        let mut request = {
            let read_guard;
            let access_token = match Request::METADATA.authentication {
                AuthScheme::AccessToken => {
                    read_guard = session.read().await;

                    if let Some(session) = read_guard.as_ref() {
                        Some(session.access_token.as_str())
                    } else {
                        return Err(Error::AuthenticationRequired);
                    }
                }
                AuthScheme::None => None,
                _ => return Err(Error::NotClientRequest),
            };

            request.try_into_http_request(&self.homeserver.to_string(), access_token)?
        };

        if let HttpMethod::POST | HttpMethod::PUT | HttpMethod::DELETE = *request.method() {
            if let Some(content_type) = content_type {
                request
                    .headers_mut()
                    .append(http::header::CONTENT_TYPE, content_type);
            }
        }

        self.inner.send_request(request).await
    }

    pub async fn upload(
        &self,
        request: create_content::Request<'_>,
    ) -> Result<create_content::Response> {
        let response = self
            .send_request(request, self.session.clone(), None)
            .await?;
        Ok(create_content::Response::try_from(response)?)
    }

    pub async fn send<Request>(&self, request: Request) -> Result<Request::IncomingResponse>
    where
        Request: OutgoingRequest,
        Error: From<FromHttpResponseError<Request::EndpointError>>,
    {
        let content_type = HeaderValue::from_static("application/json");
        let response = self
            .send_request(request, self.session.clone(), Some(content_type))
            .await?;

        trace!("Got response: {:?}", response);

        Ok(Request::IncomingResponse::try_from(response)?)
    }
}

/// Build a client with the specified configuration.
pub(crate) fn client_with_config(config: &ClientConfig) -> Result<Client> {
    let http_client = reqwest::Client::builder();

    #[cfg(not(target_arch = "wasm32"))]
    let http_client = {
        let http_client = match config.timeout {
            Some(x) => http_client.timeout(x),
            None => http_client,
        };

        let http_client = if config.disable_ssl_verification {
            http_client.danger_accept_invalid_certs(true)
        } else {
            http_client
        };

        let http_client = match &config.proxy {
            Some(p) => http_client.proxy(p.clone()),
            None => http_client,
        };

        let mut headers = reqwest::header::HeaderMap::new();

        let user_agent = match &config.user_agent {
            Some(a) => a.clone(),
            None => HeaderValue::from_str(&format!("matrix-rust-sdk {}", crate::VERSION)).unwrap(),
        };

        headers.insert(reqwest::header::USER_AGENT, user_agent);

        http_client.default_headers(headers)
    };

    #[cfg(target_arch = "wasm32")]
    #[allow(unused)]
    let _ = config;

    Ok(http_client.build()?)
}

async fn response_to_http_response(mut response: Response) -> Result<http::Response<Vec<u8>>> {
    let status = response.status();

    let mut http_builder = HttpResponse::builder().status(status);
    let headers = http_builder.headers_mut().unwrap();

    for (k, v) in response.headers_mut().drain() {
        if let Some(key) = k {
            headers.insert(key, v);
        }
    }

    let body = response.bytes().await?.as_ref().to_owned();

    Ok(http_builder.body(body).unwrap())
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl HttpSend for Client {
    async fn send_request(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>> {
        Ok(
            response_to_http_response(self.execute(reqwest::Request::try_from(request)?).await?)
                .await?,
        )
    }
}
