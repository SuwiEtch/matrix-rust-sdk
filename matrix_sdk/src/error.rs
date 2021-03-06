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

//! Error conditions.

use matrix_sdk_base::{Error as MatrixError, StoreError};
use matrix_sdk_common::{
    api::{
        r0::uiaa::{UiaaInfo, UiaaResponse as UiaaError},
        Error as RumaClientError,
    },
    FromHttpResponseError as RumaResponseError, IntoHttpError as RumaIntoHttpError, ServerError,
};
use reqwest::Error as ReqwestError;
use serde_json::Error as JsonError;
use std::io::Error as IoError;
use thiserror::Error;

#[cfg(feature = "encryption")]
use matrix_sdk_base::crypto::store::CryptoStoreError;

/// Result type of the rust-sdk.
pub type Result<T> = std::result::Result<T, Error>;

/// Internal representation of errors.
#[derive(Error, Debug)]
pub enum Error {
    /// Queried endpoint requires authentication but was called on an anonymous client.
    #[error("the queried endpoint requires authentication but was called before logging in")]
    AuthenticationRequired,

    /// Queried endpoint is not meant for clients.
    #[error("the queried endpoint is not meant for clients")]
    NotClientRequest,

    /// An error at the HTTP layer.
    #[error(transparent)]
    Reqwest(#[from] ReqwestError),

    /// An error de/serializing type for the `StateStore`
    #[error(transparent)]
    SerdeJson(#[from] JsonError),

    /// An IO error happened.
    #[error(transparent)]
    IO(#[from] IoError),

    /// An error converting between ruma_client_api types and Hyper types.
    #[error("can't parse the JSON response as a Matrix response")]
    RumaResponse(RumaResponseError<RumaClientError>),

    /// An error converting between ruma_client_api types and Hyper types.
    #[error("can't convert between ruma_client_api and hyper types.")]
    IntoHttp(RumaIntoHttpError),

    /// An error occurred in the Matrix client library.
    #[error(transparent)]
    MatrixError(#[from] MatrixError),

    /// An error occurred in the crypto store.
    #[cfg(feature = "encryption")]
    #[error(transparent)]
    CryptoStoreError(#[from] CryptoStoreError),

    /// An error occured in the state store.
    #[error(transparent)]
    StateStore(#[from] StoreError),

    /// An error occurred while authenticating.
    ///
    /// When registering or authenticating the Matrix server can send a `UiaaResponse`
    /// as the error type, this is a User-Interactive Authentication API response. This
    /// represents an error with information about how to authenticate the user.
    #[error("User-Interactive Authentication required.")]
    UiaaError(RumaResponseError<UiaaError>),
}

impl Error {
    /// Try to destructure the error into an universal interactive auth info.
    ///
    /// Some requests require universal interactive auth, doing such a request
    /// will always fail the first time with a 401 status code, the response
    /// body will contain info how the client can authenticate.
    ///
    /// The request will need to be retried, this time containing additional
    /// authentication data.
    ///
    /// This method is an convenience method to get to the info the server
    /// returned on the first, failed request.
    pub fn uiaa_response(&self) -> Option<&UiaaInfo> {
        if let Error::UiaaError(RumaResponseError::Http(ServerError::Known(
            UiaaError::AuthResponse(i),
        ))) = self
        {
            Some(i)
        } else {
            None
        }
    }
}

impl From<RumaResponseError<UiaaError>> for Error {
    fn from(error: RumaResponseError<UiaaError>) -> Self {
        Self::UiaaError(error)
    }
}

impl From<RumaResponseError<RumaClientError>> for Error {
    fn from(error: RumaResponseError<RumaClientError>) -> Self {
        Self::RumaResponse(error)
    }
}

impl From<RumaIntoHttpError> for Error {
    fn from(error: RumaIntoHttpError) -> Self {
        Self::IntoHttp(error)
    }
}
