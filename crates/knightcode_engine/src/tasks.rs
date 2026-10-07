use crate::client::{ClientError, EngineClient};
use http_client::Method;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TaskStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Cancelling => "Cancelling",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    pub enabled: bool,
    pub max_concurrent: u32,
    pub agents: Vec<AgentProfile>,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub text: String,
    pub tool_calls: u64,
    pub last_tool: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub parent_session_id: String,
    pub session_id: String,
    pub run_id: String,
    pub revision: u64,
    pub status: TaskStatus,
    pub agent: String,
    pub prompt: String,
    pub model: String,
    pub thinking_level: String,
    pub cwd: String,
    pub tool_call_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub progress: TaskProgress,
    pub usage: TaskUsage,
    pub result: Option<String>,
    #[serde(default)]
    pub result_truncated: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Tasks {
    tasks: Vec<TaskSnapshot>,
}

fn path_segment(id: &str) -> String {
    let mut encoded = String::new();
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

impl EngineClient {
    pub async fn subagent_config(&self, session_id: &str) -> Result<SubagentConfig, ClientError> {
        self.call(
            Method::GET,
            &format!("/v1/sessions/{}/subagents/config", path_segment(session_id)),
            None,
        )
        .await
    }

    pub async fn set_subagent_config(
        &self,
        session_id: &str,
        config: &SubagentConfig,
    ) -> Result<SubagentConfig, ClientError> {
        self.call(
            Method::PUT,
            &format!("/v1/sessions/{}/subagents/config", path_segment(session_id)),
            Some(serde_json::to_value(config).map_err(anyhow::Error::from)?),
        )
        .await
    }

    pub async fn tasks(&self, session_id: &str) -> Result<Vec<TaskSnapshot>, ClientError> {
        Ok(self
            .call::<Tasks>(
                Method::GET,
                &format!("/v1/sessions/{}/tasks", path_segment(session_id)),
                None,
            )
            .await?
            .tasks)
    }

    pub async fn spawn_tasks(
        &self,
        session_id: &str,
        tasks: &[TaskInput],
        background: bool,
    ) -> Result<Vec<TaskSnapshot>, ClientError> {
        Ok(self
            .call::<Tasks>(
                Method::POST,
                &format!("/v1/sessions/{}/tasks", path_segment(session_id)),
                Some(serde_json::json!({ "tasks": tasks, "background": background })),
            )
            .await?
            .tasks)
    }

    pub async fn task_snapshot(&self, task_id: &str) -> Result<TaskSnapshot, ClientError> {
        self.call(
            Method::GET,
            &format!("/v1/tasks/{}", path_segment(task_id)),
            None,
        )
        .await
    }

    pub async fn resume_task(
        &self,
        task_id: &str,
        input: &TaskInput,
    ) -> Result<TaskSnapshot, ClientError> {
        let mut body = serde_json::json!({ "prompt": input.prompt });
        if let Some(model) = &input.model {
            body["model"] = model.clone().into();
        }
        if let Some(thinking_level) = &input.thinking_level {
            body["thinkingLevel"] = thinking_level.clone().into();
        }
        self.call(
            Method::POST,
            &format!("/v1/tasks/{}/resume", path_segment(task_id)),
            Some(body),
        )
        .await
    }

    pub async fn cancel_task(&self, task_id: &str) -> Result<TaskSnapshot, ClientError> {
        self.call(
            Method::POST,
            &format!("/v1/tasks/{}/cancel", path_segment(task_id)),
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Endpoint, EngineEvent};
    use futures::AsyncReadExt as _;
    use http_client::{AsyncBody, FakeHttpClient, Response};
    use std::sync::{Arc, Mutex};

    const SNAPSHOT: &str = r#"{"id":"task/1","parentSessionId":"parent/1","sessionId":"child","runId":"run","revision":12,"status":"interrupted","agent":"worker","prompt":"Inspect","model":"provider/model","thinkingLevel":"high","cwd":"C:/project","createdAt":"2026-10-04T00:00:00Z","updatedAt":"2026-10-04T00:01:00Z","progress":{"text":"Reading","toolCalls":2,"lastTool":"read"},"usage":{"input":10,"output":20,"cacheRead":30,"cacheWrite":40,"total":100,"cost":0.01},"error":"Connection lost"}"#;

    #[test]
    fn task_routes_preserve_inheritance_revision_and_bodyless_cancel() -> anyhow::Result<()> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let http = FakeHttpClient::create({
            let seen = seen.clone();
            move |mut request| {
                let seen = seen.clone();
                async move {
                    assert_eq!(
                        request
                            .headers()
                            .get("Authorization")
                            .and_then(|header| header.to_str().ok()),
                        Some("Bearer token")
                    );
                    let path = request.uri().path().to_owned();
                    let method = request.method().to_string();
                    let mut text = String::new();
                    request.body_mut().read_to_string(&mut text).await?;
                    let body = if text.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::from_str(&text)?
                    };
                    seen.lock()
                        .map_err(|error| anyhow::anyhow!("{error}"))?
                        .push((method.clone(), path.clone(), body));
                    let response = if path.ends_with("/config") {
                        r#"{"enabled":false,"maxConcurrent":4,"agents":[],"revision":3}"#.to_owned()
                    } else if path.ends_with("/tasks") {
                        format!("{{\"tasks\":[{SNAPSHOT}]}}")
                    } else {
                        SNAPSHOT.to_owned()
                    };
                    Ok(Response::builder()
                        .status(if method == "POST" && !path.ends_with("/cancel") {
                            202
                        } else {
                            200
                        })
                        .body(AsyncBody::from(response))?)
                }
            }
        });
        let client = EngineClient::new(
            http,
            Endpoint {
                url: "http://127.0.0.1:1".into(),
                token: "token".into(),
            },
        );
        smol::block_on(async {
            let config = client.subagent_config("parent/1").await?;
            assert!(!config.enabled);
            assert_eq!(
                client
                    .set_subagent_config("parent/1", &config)
                    .await?
                    .revision,
                3
            );
            assert_eq!(client.tasks("parent/1").await?.len(), 1);
            client
                .spawn_tasks(
                    "parent/1",
                    &[TaskInput {
                        prompt: "Inspect".into(),
                        ..Default::default()
                    }],
                    true,
                )
                .await?;
            assert_eq!(
                client.task_snapshot("task/1").await?.status,
                TaskStatus::Interrupted
            );
            client
                .resume_task(
                    "task/1",
                    &TaskInput {
                        agent: Some("ignored-on-resume".into()),
                        prompt: "Continue".into(),
                        model: Some("local/actual".into()),
                        thinking_level: Some("max".into()),
                    },
                )
                .await?;
            client.cancel_task("task/1").await?;
            anyhow::Ok(())
        })?;
        let seen = seen.lock().map_err(|error| anyhow::anyhow!("{error}"))?;
        assert_eq!(seen.len(), 7);
        let mut requests = seen.iter();
        let (_, path, _) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing config GET"))?;
        assert_eq!(path, "/v1/sessions/parent%2F1/subagents/config");
        let (method, _, body) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing config PUT"))?;
        assert_eq!(method, "PUT");
        assert_eq!(
            body,
            &serde_json::json!({ "enabled": false, "maxConcurrent": 4, "agents": [], "revision": 3 })
        );
        requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing list"))?;
        let (_, _, body) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing dispatch"))?;
        assert_eq!(
            body,
            &serde_json::json!({ "tasks": [{ "prompt": "Inspect" }], "background": true })
        );
        let (_, path, _) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
        assert_eq!(path, "/v1/tasks/task%2F1");
        let (_, path, body) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing resume"))?;
        assert_eq!(path, "/v1/tasks/task%2F1/resume");
        assert_eq!(
            body,
            &serde_json::json!({ "prompt": "Continue", "model": "local/actual", "thinkingLevel": "max" })
        );
        let (method, path, body) = requests
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing cancel"))?;
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("POST", "/v1/tasks/task%2F1/cancel")
        );
        assert!(body.is_null());
        Ok(())
    }

    #[test]
    fn task_changed_sse_keeps_the_full_snapshot_and_revision() -> anyhow::Result<()> {
        let stream = format!(
            "event: task.changed\ndata: {{\"type\":\"task.changed\",\"task\":{SNAPSHOT}}}\n\n"
        );
        let http = FakeHttpClient::create(move |_| {
            let stream = stream.clone();
            async move {
                Ok(Response::builder()
                    .status(200)
                    .body(AsyncBody::from(stream))?)
            }
        });
        let client = EngineClient::new(
            http,
            Endpoint {
                url: "http://127.0.0.1:1".into(),
                token: "token".into(),
            },
        );
        let mut events = Vec::new();
        smol::block_on(client.events(|event| events.push(event)))?;
        assert!(matches!(events.first(), Some(EngineEvent::EventsConnected)));
        let Some(EngineEvent::TaskChanged { task }) = events.get(1) else {
            anyhow::bail!("missing typed task event");
        };
        assert_eq!(task.revision, 12);
        assert_eq!(task.usage.cache_write, 40);
        assert_eq!(task.cwd, "C:/project");
        assert_eq!(task.error.as_deref(), Some("Connection lost"));
        Ok(())
    }
}
