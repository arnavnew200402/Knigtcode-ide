use anyhow::anyhow;
use http_client::Method;
use serde::{Deserialize, Serialize};

use crate::client::{ClientError, EngineClient};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    Chat,
    Plan,
    Build,
}

impl SessionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Plan => "plan",
            Self::Build => "build",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSubagent {
    pub task_id: String,
    pub parent_session_id: String,
    pub agent: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionControls {
    pub id: String,
    pub cwd: String,
    pub mode: SessionMode,
    pub web_research: bool,
    #[serde(default)]
    pub subagent: Option<SessionSubagent>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPolicyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<SessionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_research: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionHandoff {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionHandoffResponse {
    pub session: SessionControls,
    pub messages: Vec<serde_json::Value>,
}

fn session_path(session_id: &str) -> Result<String, ClientError> {
    if session_id.is_empty() {
        return Err(anyhow!("session id is required").into());
    }
    let mut encoded = String::new();
    for byte in session_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(format!("/v1/sessions/{encoded}"))
}

impl EngineClient {
    pub async fn session_controls(&self, session_id: &str) -> Result<SessionControls, ClientError> {
        let controls: SessionControls = self
            .call(Method::GET, &session_path(session_id)?, None)
            .await?;
        if controls.id != session_id {
            return Err(anyhow!("the engine returned a different session").into());
        }
        Ok(controls)
    }

    pub async fn update_session_policy(
        &self,
        session_id: &str,
        patch: &SessionPolicyPatch,
    ) -> Result<SessionControls, ClientError> {
        let body = serde_json::to_value(patch).map_err(anyhow::Error::from)?;
        let controls: SessionControls = self
            .call(Method::PATCH, &session_path(session_id)?, Some(body))
            .await?;
        if controls.id != session_id {
            return Err(anyhow!("the engine returned a different session").into());
        }
        if patch.mode.is_some_and(|mode| controls.mode != mode) {
            return Err(anyhow!("the engine did not apply the requested mode").into());
        }
        if patch
            .web_research
            .is_some_and(|enabled| controls.web_research != enabled)
        {
            return Err(anyhow!("the engine did not apply web research").into());
        }
        Ok(controls)
    }

    pub async fn handoff_session(
        &self,
        session_id: &str,
        handoff: &SessionHandoff,
    ) -> Result<SessionHandoffResponse, ClientError> {
        if handoff.cwd.trim().is_empty() {
            return Err(anyhow!("a project directory is required for handoff").into());
        }
        let body = serde_json::to_value(handoff).map_err(anyhow::Error::from)?;
        let response: SessionHandoffResponse = self
            .call(
                Method::POST,
                &format!("{}/handoff", session_path(session_id)?),
                Some(body),
            )
            .await?;
        if response.session.id != session_id {
            return Err(anyhow!("handoff returned a different session").into());
        }
        if response.session.mode != SessionMode::Build {
            return Err(anyhow!("handoff did not enter Build mode").into());
        }
        if response.session.subagent.is_some() {
            return Err(anyhow!("handoff returned a child session").into());
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_and_handoff_payloads_match_engine_routes() -> anyhow::Result<()> {
        let policy = serde_json::to_value(SessionPolicyPatch {
            mode: Some(SessionMode::Chat),
            web_research: Some(true),
        })?;
        assert_eq!(
            policy,
            serde_json::json!({ "mode": "chat", "webResearch": true })
        );

        let handoff = serde_json::to_value(SessionHandoff {
            cwd: "C:/work/project".into(),
            text: Some("edited intent".into()),
            plan: Some("edited plan".into()),
        })?;
        assert_eq!(
            handoff,
            serde_json::json!({
                "cwd": "C:/work/project",
                "text": "edited intent",
                "plan": "edited plan"
            })
        );
        assert_eq!(session_path("session/id")?, "/v1/sessions/session%2Fid");
        Ok(())
    }
}
