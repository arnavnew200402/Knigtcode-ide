use anyhow::anyhow;
use http_client::Method;
use serde::{Deserialize, Serialize};

use crate::client::{ClientError, EngineClient, LoginOption};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LocalProviderKind {
    Ollama,
    Lmstudio,
    Llamacpp,
    Openai,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub context_window: u64,
    pub max_tokens: u64,
    pub reasoning: bool,
    pub input: Vec<String>,
    pub cost: LocalModelCost,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalProviderConfig {
    pub kind: LocalProviderKind,
    pub name: String,
    pub base_url: String,
    pub models: Vec<LocalModel>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub configured: bool,
    #[serde(rename = "loginOptions")]
    pub login_options: Vec<LoginOption>,
    pub local: Option<LocalProviderConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalProviderPreset {
    pub id: String,
    pub name: String,
    pub kind: LocalProviderKind,
    pub base_url: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Providers {
    pub providers: Vec<ProviderSummary>,
    pub presets: Vec<LocalProviderPreset>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredLocalModel {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub reasoning: Option<bool>,
    pub input: Option<Vec<String>>,
    pub cost: Option<LocalModelCost>,
}

#[derive(Deserialize)]
struct DiscoveredModels {
    models: Vec<DiscoveredLocalModel>,
}

fn provider_path(provider_id: &str) -> Result<String, ClientError> {
    if provider_id.is_empty()
        || !provider_id.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
    {
        return Err(anyhow!("invalid provider id").into());
    }
    Ok(format!("/v1/providers/{provider_id}"))
}

impl EngineClient {
    pub async fn providers(&self) -> Result<Providers, ClientError> {
        self.call(Method::GET, "/v1/providers", None).await
    }

    pub async fn discover_local_models(
        &self,
        kind: LocalProviderKind,
        base_url: &str,
        provider_id: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<DiscoveredLocalModel>, ClientError> {
        let mut body = serde_json::json!({ "kind": kind, "baseUrl": base_url });
        if let Some(provider_id) = provider_id {
            body["providerId"] = provider_id.into();
        }
        if let Some(api_key) = api_key {
            body["apiKey"] = api_key.into();
        }
        Ok(self
            .call::<DiscoveredModels>(Method::POST, "/v1/providers/discover", Some(body))
            .await?
            .models)
    }

    pub async fn save_local_provider(
        &self,
        provider_id: &str,
        config: &LocalProviderConfig,
        api_key: Option<&str>,
    ) -> Result<(), ClientError> {
        let mut body = serde_json::to_value(config).map_err(anyhow::Error::from)?;
        if let Some(api_key) = api_key {
            body["apiKey"] = api_key.into();
        }
        self.call::<serde_json::Value>(Method::PUT, &provider_path(provider_id)?, Some(body))
            .await?;
        Ok(())
    }

    pub async fn remove_local_provider(&self, provider_id: &str) -> Result<(), ClientError> {
        self.call::<()>(Method::DELETE, &provider_path(provider_id)?, None)
            .await
    }

    pub async fn refresh_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<DiscoveredLocalModel>, ClientError> {
        Ok(self
            .call::<DiscoveredModels>(
                Method::POST,
                &format!("{}/refresh", provider_path(provider_id)?),
                None,
            )
            .await?
            .models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Endpoint;
    use futures::AsyncReadExt as _;
    use http_client::{AsyncBody, FakeHttpClient, Response};
    use std::sync::{Arc, Mutex};

    #[test]
    fn model_list_does_not_imply_capabilities() -> anyhow::Result<()> {
        let models: DiscoveredModels =
            serde_json::from_str(r#"{"models":[{"id":"vision-thinking-name"}]}"#)?;
        let model = models
            .models
            .first()
            .ok_or_else(|| anyhow!("missing model"))?;
        assert_eq!(model.context_window, None);
        assert_eq!(model.max_tokens, None);
        assert_eq!(model.reasoning, None);
        assert_eq!(model.input, None);
        Ok(())
    }

    #[test]
    fn provider_response_has_no_credential_fields() -> anyhow::Result<()> {
        let providers: Providers = serde_json::from_str(
            r#"{"providers":[{"id":"ollama","name":"Ollama","kind":"ollama","configured":true,"loginOptions":[],"local":{"kind":"ollama","name":"Ollama","baseUrl":"http://localhost:11434/v1","models":[]}}],"presets":[]}"#,
        )?;
        assert_eq!(providers.providers.len(), 1);
        assert!(
            providers
                .providers
                .first()
                .is_some_and(|provider| provider.local.is_some())
        );
        assert!(provider_path("../../accounts").is_err());
        Ok(())
    }

    #[test]
    fn provider_requests_preserve_keep_replace_and_clear_key_semantics() -> anyhow::Result<()> {
        let requests = Arc::new(Mutex::new(Vec::<(String, String, serde_json::Value)>::new()));
        let http = FakeHttpClient::create({
            let requests = requests.clone();
            move |mut request| {
                let requests = requests.clone();
                async move {
                    assert_eq!(
                        request
                            .headers()
                            .get("Authorization")
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer engine-token")
                    );
                    let method = request.method().to_string();
                    let path = request.uri().path().to_owned();
                    let mut text = String::new();
                    request.body_mut().read_to_string(&mut text).await?;
                    let body = if text.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::from_str(&text)?
                    };
                    requests.lock().map_err(|error| anyhow!("{error}"))?.push((
                        method.clone(),
                        path.clone(),
                        body,
                    ));
                    let body = if method == "DELETE" {
                        ""
                    } else if path.ends_with("/refresh") || path.ends_with("/discover") {
                        r#"{"models":[{"id":"actual"}]}"#
                    } else {
                        "{}"
                    };
                    Ok(Response::builder()
                        .status(if method == "DELETE" { 204 } else { 200 })
                        .body(AsyncBody::from(body))?)
                }
            }
        });
        let client = EngineClient::new(
            http,
            Endpoint {
                url: "http://127.0.0.1:1".into(),
                token: "engine-token".into(),
            },
        );
        let config = LocalProviderConfig {
            kind: LocalProviderKind::Ollama,
            name: "Ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            models: vec![],
        };
        smol::block_on(async {
            client.save_local_provider("ollama", &config, None).await?;
            client
                .save_local_provider("ollama", &config, Some("secret-write-only"))
                .await?;
            client
                .save_local_provider("ollama", &config, Some(""))
                .await?;
            client
                .discover_local_models(config.kind, &config.base_url, Some("ollama"), None)
                .await?;
            client.refresh_provider_models("ollama").await?;
            client.remove_local_provider("ollama").await?;
            anyhow::Ok(())
        })?;
        let requests = requests.lock().map_err(|error| anyhow!("{error}"))?;
        let mut requests = requests.iter();
        let (_, _, keep) = requests
            .next()
            .ok_or_else(|| anyhow!("missing keep request"))?;
        assert!(keep.get("apiKey").is_none());
        let (_, _, replace) = requests
            .next()
            .ok_or_else(|| anyhow!("missing replace request"))?;
        assert_eq!(
            replace.get("apiKey").and_then(|value| value.as_str()),
            Some("secret-write-only")
        );
        let (_, _, clear) = requests
            .next()
            .ok_or_else(|| anyhow!("missing clear request"))?;
        assert_eq!(
            clear.get("apiKey").and_then(|value| value.as_str()),
            Some("")
        );
        let (method, path, discovery) = requests
            .next()
            .ok_or_else(|| anyhow!("missing discovery request"))?;
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("POST", "/v1/providers/discover")
        );
        assert_eq!(
            discovery.get("providerId").and_then(|value| value.as_str()),
            Some("ollama")
        );
        assert!(discovery.get("apiKey").is_none());
        let (method, path, _) = requests
            .next()
            .ok_or_else(|| anyhow!("missing refresh request"))?;
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("POST", "/v1/providers/ollama/refresh")
        );
        let (method, path, _) = requests
            .next()
            .ok_or_else(|| anyhow!("missing remove request"))?;
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("DELETE", "/v1/providers/ollama")
        );
        Ok(())
    }
}
