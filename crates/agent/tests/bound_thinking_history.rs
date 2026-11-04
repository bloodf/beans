use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beans_agent::harness::{AgentHarness, HarnessEvent, HarnessOptions, Outcome};
use beans_agent::provider::channel_stream;
use beans_agent::providers::anthropic::drop_bound_thinking;
use beans_agent::{
    Agent, AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AgentOptions, AssistantEvent, AssistantEventStream,
    AssistantMessage, AssistantPart, LlmMessage, ModelRequest, Provider, StopReason, ToolCall,
    ToolResultMessage, Usage, EventSink, run_agent_loop,
};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const MODEL: &str = "claude-opus-5-5";
const ERROR: &str = "Invalid signature in thinking block. The block is bound to a different conversation.";

fn rejected_parts() -> Vec<AssistantPart> {
    vec![
        AssistantPart::Thinking { thinking: "old reasoning".into(), signature: Some("rejected".into()) },
        AssistantPart::ServerBlock { block: json!({"type": "redacted_thinking", "data": "rejected"}) },
        AssistantPart::ServerBlock { block: json!({"type": "server_tool_use", "id": "search", "name": "web_search", "input": {}}) },
        AssistantPart::ServerBlock { block: json!({"type": "web_search_tool_result", "tool_use_id": "search", "content": []}) },
        AssistantPart::Text { text: "Earlier answer".into() },
        AssistantPart::ToolCall(ToolCall { id: "call".into(), name: "missing".into(), arguments: json!({"keep": true}) }),
    ]
}

fn history(generated_in_run: bool) -> Vec<AgentMessage> {
    let mut messages = vec![
        AgentMessage::user("original prompt"),
        AgentMessage::Custom { kind: "host_note".into(), data: json!({"keep": "unchanged"}), timestamp: 1 },
    ];
    if !generated_in_run {
        let mut earlier = AssistantMessage::empty("anthropic", MODEL);
        earlier.content = rejected_parts();
        earlier.stop_reason = StopReason::ToolUse;
        messages.push(AgentMessage::Assistant(earlier));
        messages.push(AgentMessage::ToolResult(ToolResultMessage {
            tool_call_id: "call".into(), tool_name: "missing".into(),
            content: vec![beans_agent::ContentPart::text("original result")],
            details: json!({"keep": "details"}), is_error: false, timestamp: 2,
        }));
    }
    messages
}

struct RejectsThinking {
    requests: Mutex<Vec<ModelRequest>>,
    generated_in_run: bool,
}

#[async_trait]
impl Provider for RejectsThinking {
    fn provider_id(&self) -> &str { "anthropic" }
    fn model_id(&self) -> &str { MODEL }

    async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
        let rejected = request.messages.iter().any(|message| {
            matches!(message, LlmMessage::Assistant(assistant) if assistant.content.iter().any(|part| {
                matches!(part, AssistantPart::Thinking { signature: Some(signature), .. } if signature == "rejected")
                    || matches!(part, AssistantPart::ServerBlock { block } if block["type"] == "redacted_thinking" && block["data"] == "rejected")
            }))
        });
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len() == 1
        };
        let generated_in_run = self.generated_in_run;
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let _ = tx.send(AssistantEvent::Start).await;
            if rejected {
                let _ = tx.send(AssistantEvent::Error { message: ERROR.into(), aborted: false }).await;
            } else if first && generated_in_run {
                for (index, part) in rejected_parts().into_iter().enumerate() {
                    match part {
                        AssistantPart::Thinking { thinking, signature } => {
                            let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                            let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: thinking }).await;
                            let _ = tx.send(AssistantEvent::ThinkingSignature { index, signature: signature.unwrap() }).await;
                        }
                        AssistantPart::ServerBlock { block } => {
                            let _ = tx.send(AssistantEvent::ServerBlock { index, block }).await;
                        }
                        AssistantPart::Text { text } => {
                            let _ = tx.send(AssistantEvent::TextStart { index }).await;
                            let _ = tx.send(AssistantEvent::TextDelta { index, delta: text }).await;
                        }
                        AssistantPart::ToolCall(call) => {
                            let _ = tx.send(AssistantEvent::ToolCallStart { index, id: call.id, name: call.name }).await;
                            let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: call.arguments.to_string() }).await;
                            let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
                        }
                    }
                }
                let _ = tx.send(AssistantEvent::Done { stop_reason: StopReason::ToolUse, usage: Usage::default() }).await;
            } else {
                let _ = tx.send(AssistantEvent::ThinkingStart { index: 0 }).await;
                let _ = tx.send(AssistantEvent::ThinkingDelta { index: 0, delta: "new reasoning".into() }).await;
                let _ = tx.send(AssistantEvent::ThinkingSignature { index: 0, signature: "accepted".into() }).await;
                let _ = tx.send(AssistantEvent::TextStart { index: 1 }).await;
                let _ = tx.send(AssistantEvent::TextDelta { index: 1, delta: "done".into() }).await;
                let _ = tx.send(AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Usage::default() }).await;
            }
        });
        channel_stream(rx)
    }
}

fn assert_repaired(provider: &RejectsThinking, original: &[AgentMessage], messages: &[AgentMessage]) {
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), if provider.generated_in_run { 4 } else { 3 }, "the second prompt never repeats the rejected request");
    let mut expected_prefix = original.to_vec();
    drop_bound_thinking(MODEL, &mut expected_prefix);
    assert_eq!(&messages[..original.len()], expected_prefix, "all other history stays byte-for-byte equivalent");
    let repaired = messages.iter().find_map(|message| match message {
        AgentMessage::Assistant(assistant) if !assistant.tool_calls().is_empty() => Some(assistant),
        _ => None,
    }).unwrap();
    assert_eq!(repaired.content, rejected_parts()[2..], "text, tool calls, and server calls/results survive");
    let Some(AgentMessage::Assistant(last)) = messages.last() else { panic!() };
    assert_eq!(last.stop_reason, StopReason::Stop);
    assert!(matches!(&last.content[0], AssistantPart::Thinking { signature: Some(signature), .. } if signature == "accepted"), "new thinking is not stripped");
    let last_request = requests.last().unwrap();
    assert!(last_request.messages.iter().any(|message| matches!(message, LlmMessage::Assistant(assistant) if assistant.content.iter().any(|part| matches!(part, AssistantPart::Thinking { signature: Some(signature), .. } if signature == "accepted")))), "the next prompt replays newly accepted thinking");
}

#[tokio::test]
async fn agent_keeps_bound_thinking_repair_across_prompts_and_in_returned_messages() {
    for generated_in_run in [false, true] {
        let provider = Arc::new(RejectsThinking { requests: Mutex::new(vec![]), generated_in_run });
        let original = history(generated_in_run);
        let mut options = AgentOptions::new(provider.clone());
        options.messages = original.clone();
        let mut agent = Agent::new(options);
        let mut stored = original.clone();
        let mut returned = Vec::new();
        let mut repairs = 0;
        for prompt in ["first prompt", "second prompt"] {
            let (tx, mut rx) = mpsc::channel(128);
            let collector = tokio::spawn(async move {
                let mut events = Vec::new();
                while let Some(event) = rx.recv().await { events.push(event); }
                events
            });
            returned.extend(agent.prompt(prompt, tx).await.unwrap());
            for event in collector.await.unwrap() {
                match event {
                    AgentEvent::MessageEnd { message } => stored.push(message),
                    AgentEvent::BoundThinkingDropped { model } => {
                        repairs += 1;
                        drop_bound_thinking(&model, &mut stored);
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(repairs, 1);
        assert_repaired(&provider, &original, &agent.messages);
        assert_eq!(stored, agent.messages, "event consumers see the same repaired history");
        assert_eq!(returned, agent.messages[original.len()..], "the run does not return stale thinking");
    }
}

#[tokio::test]
async fn harness_keeps_bound_thinking_repair_across_prompts_and_in_returned_messages() {
    for generated_in_run in [false, true] {
        let provider = Arc::new(RejectsThinking { requests: Mutex::new(vec![]), generated_in_run });
        let original = history(generated_in_run);
        let mut options = HarnessOptions::new(provider.clone());
        options.messages = original.clone();
        let mut harness = AgentHarness::new(options);
        let mut rx = harness.subscribe();
        let collector = tokio::spawn(async move {
            let mut events = Vec::new();
            while let Some(event) = rx.recv().await { events.push(event); }
            events
        });
        let mut returned = Vec::new();
        for prompt in ["first prompt", "second prompt"] {
            let result = harness.prompt(prompt).await.unwrap();
            assert_eq!(result.outcome, Outcome::Completed);
            returned.extend(result.messages);
        }
        let messages = harness.messages().to_vec();
        drop(harness);
        let mut stored = original.clone();
        let mut repairs = 0;
        for event in collector.await.unwrap() {
            match event {
                HarnessEvent::MessageEnd { message, .. } => stored.push(message),
                HarnessEvent::BoundThinkingDropped { model, .. } => {
                    repairs += 1;
                    drop_bound_thinking(&model, &mut stored);
                }
                _ => {}
            }
        }
        assert_eq!(repairs, 1);
        assert_repaired(&provider, &original, &messages);
        assert_eq!(stored, messages, "harness listeners can persist the repair");
        assert_eq!(returned, messages[original.len()..], "RunResult does not return stale thinking");
    }
}

#[tokio::test]
async fn cancellation_while_persisting_repair_never_starts_another_request() {
    struct CancelOnRepair(CancellationToken);
    #[async_trait]
    impl EventSink for CancelOnRepair {
        async fn on_event(&self, event: &AgentEvent) {
            if matches!(event, AgentEvent::BoundThinkingDropped { .. }) {
                self.0.cancel();
            }
        }
    }
    let provider = Arc::new(RejectsThinking { requests: Mutex::new(vec![]), generated_in_run: false });
    let cancel = CancellationToken::new();
    let context = AgentContext { system_prompt: String::new(), messages: history(false), tools: vec![], cache_points: vec![] };
    let config = AgentLoopConfig::new(provider.clone()).with_sink(Arc::new(CancelOnRepair(cancel.clone())));
    let (tx, rx) = mpsc::channel(32);
    drop(rx);
    let result = run_agent_loop(vec![AgentMessage::user("go")], context, &config, &tx, cancel).await;
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    assert_eq!(result.len(), 2, "the prompt and the final aborted message, without the empty partial");
    let Some(AgentMessage::Assistant(last)) = result.last() else { panic!() };
    assert_eq!(last.stop_reason, StopReason::Aborted);
    assert_eq!(last.error_message.as_deref(), Some("Request aborted"));
}
