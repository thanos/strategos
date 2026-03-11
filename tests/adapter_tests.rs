use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use strategos::adapters::fake::{FakeAdapter, FakeBehavior};
use strategos::adapters::traits::*;
use strategos::adapters::claude::ClaudeAdapter;
use strategos::adapters::claude::ClaudeConfig;
use strategos::adapters::ollama::OllamaAdapter;
use strategos::adapters::ollama::OllamaConfig;
use strategos::adapters::opencode::OpenCodeAdapter;
use strategos::adapters::opencode::OpenCodeConfig;
use strategos::errors::AdapterError;
use strategos::models::{BackendId, MoneyAmount, TaskId, TaskType};

fn test_request() -> ExecutionRequest {
    ExecutionRequest {
        task_id: TaskId::new(),
        task_type: TaskType::Summarization,
        prompt: "summarize this code".into(),
        context: ExecutionContext {
            project_path: PathBuf::from("/tmp/test-project"),
            working_directory: None,
            files: Vec::new(),
            session_id: None,
            metadata: HashMap::new(),
        },
        constraints: ExecutionConstraints::default(),
    }
}

#[test]
fn registry_manages_multiple_adapters() {
    let mut registry = AdapterRegistry::new();

    let claude = Arc::new(FakeAdapter::succeeding("claude"));
    let ollama = Arc::new(FakeAdapter::local("ollama"));

    registry.register(claude);
    registry.register(ollama);

    assert_eq!(registry.list().len(), 2);
    assert!(registry.is_available(&BackendId::new("claude")));
    assert!(registry.is_available(&BackendId::new("ollama")));
    assert!(!registry.is_available(&BackendId::new("unknown")));
}

#[test]
fn claude_adapter_reports_correct_capabilities() {
    let adapter = ClaudeAdapter::new(ClaudeConfig::default());
    let caps = adapter.capabilities();
    assert!(caps.code_editing);
    assert!(caps.shell_tool_use);
    assert!(caps.multi_step_agent);
    assert!(!caps.local_execution);
    assert!(caps.supports_task_type(&TaskType::DeepCodeReasoning));
    assert!(caps.supports_task_type(&TaskType::Planning));
    assert!(!caps.supports_task_type(&TaskType::PrivateLocalTask));
}

#[test]
fn ollama_adapter_reports_correct_capabilities() {
    let adapter = OllamaAdapter::new(OllamaConfig::default());
    let caps = adapter.capabilities();
    assert!(caps.local_execution);
    assert!(!caps.code_editing);
    assert!(caps.supports_task_type(&TaskType::PrivateLocalTask));
    assert!(caps.supports_task_type(&TaskType::Summarization));
    assert!(!caps.supports_task_type(&TaskType::DeepCodeReasoning));
}

#[test]
fn opencode_adapter_reports_correct_capabilities() {
    let adapter = OpenCodeAdapter::new(OpenCodeConfig::default());
    let caps = adapter.capabilities();
    assert!(caps.code_editing);
    assert!(caps.multi_step_agent);
    assert!(!caps.local_execution);
    assert!(caps.supports_task_type(&TaskType::Experimental));
}

#[tokio::test]
async fn fake_adapter_succeeds_with_usage() {
    let adapter = FakeAdapter::new(
        "test",
        FakeAdapter::succeeding("x").capabilities().clone(),
        FakeBehavior::SucceedWithUsage {
            output: "result".into(),
            input_tokens: 500,
            output_tokens: 200,
            cost: MoneyAmount::from_cents(150),
        },
    );

    let handle = adapter.submit(test_request()).await.unwrap();
    let status = adapter.poll(&handle).await.unwrap();

    match status {
        ExecutionStatus::Completed(result) => {
            assert_eq!(result.output, "result");
            assert_eq!(result.usage.input_tokens, 500);
            assert_eq!(result.usage.output_tokens, 200);
            assert_eq!(result.usage.cost.cents, 150);
        }
        _ => panic!("expected Completed status"),
    }
}

#[tokio::test]
async fn fake_adapter_fails() {
    let adapter = FakeAdapter::failing("test", AdapterError::Unavailable("maintenance".into()));
    let result = adapter.submit(test_request()).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        AdapterError::Unavailable(msg) => assert_eq!(msg, "maintenance"),
        other => panic!("expected Unavailable, got {:?}", other),
    }
}

#[tokio::test]
async fn skeleton_adapters_return_unsupported() {
    // OpenCode is still a skeleton
    let opencode = OpenCodeAdapter::new(OpenCodeConfig::default());
    assert!(matches!(
        opencode.submit(test_request()).await,
        Err(AdapterError::Unsupported(_))
    ));
}

#[tokio::test]
async fn claude_adapter_returns_auth_error_without_key() {
    let claude = ClaudeAdapter::new(ClaudeConfig {
        api_key_env: "STRATEGOS_TEST_NONEXISTENT_KEY".into(),
        ..ClaudeConfig::default()
    });
    assert!(matches!(
        claude.submit(test_request()).await,
        Err(AdapterError::AuthenticationFailed(_))
    ));
}

#[tokio::test]
async fn ollama_adapter_returns_error_without_server() {
    let ollama = OllamaAdapter::new(OllamaConfig::default());
    let result = ollama.submit(test_request()).await;
    assert!(result.is_err());
}
