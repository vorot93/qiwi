use anyhow::format_err;
use headers::*;
use http::Method;
use reqwest_ext::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use tracing::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum Rsp<T> {
    Error {
        #[serde(rename = "errorCode")]
        error: String,
    },
    OK(T),
}

impl<T> Rsp<T> {
    pub fn into_result(self) -> anyhow::Result<T> {
        match self {
            Self::Error { error } => Err(format_err!("qiwi error: {error}")),
            Self::OK(v) => Ok(v),
        }
    }
}

pub trait Transport: Debug + Send + Sync + 'static {
    fn call(
        &self,
        endpoint: String,
        method: Method,
        params: &HashMap<&str, String>,
        body: Option<&Value>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'static>>;
}

#[derive(Debug)]
pub struct RemoteCaller {
    pub http_client: reqwest::Client,
    pub addr: String,
    pub bearer: Option<String>,
}

impl Transport for RemoteCaller {
    fn call(
        &self,
        endpoint: String,
        method: Method,
        params: &HashMap<&str, String>,
        body: Option<&Value>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'static>> {
        let client = self.http_client.clone();
        let uri = format!("{}/{}", self.addr, endpoint);
        trace!(
            "Sending request to endpoint {} with params: {:?}",
            endpoint,
            params
        );

        let mut req = client
            .request(method, uri)
            .query(params)
            .typed_header(ContentType::json());
        if let Some(bearer) = self.bearer.as_ref() {
            req = req.bearer_auth(bearer);
        }

        if let Some(body) = body {
            req = req.json(body);
        }

        Box::pin(async move {
            let rsp = req.send().await?;
            let err = rsp.error_for_status_ref().err();

            let data = rsp.text().await?;

            trace!("Received HTTP response: {data}");

            if let Some(err) = err {
                return Err(format_err!("Received error {err} with data: {data}"));
            }

            Ok(data)
        })
    }
}

#[derive(Clone, Debug)]
pub struct CallerWrapper {
    pub transport: Arc<dyn Transport>,
}

impl CallerWrapper {
    pub fn call<E, T>(
        &self,
        endpoint: E,
        method: Method,
        params: &HashMap<&str, String>,
        body: Option<&Value>,
    ) -> impl Future<Output = anyhow::Result<Rsp<T>>> + Send + 'static
    where
        E: Display,
        T: for<'de> Deserialize<'de> + Send + 'static,
    {
        let c = self
            .transport
            .call(endpoint.to_string(), method, params, body);
        async move { Ok(serde_json::from_str(&c.await?)?) }
    }
}
