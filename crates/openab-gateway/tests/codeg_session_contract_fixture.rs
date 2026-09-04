//! Executable production-contract checks for the Codeg/OpenAB session fixture.
//!
//! The session control-plane routes are covered by the admin API integration
//! tests, while this test keeps the frozen fixture aligned with production.
//! Existing payloads are decoded through public production types, while the
//! transcript/SSE trace is reproduced through the real session stores.

use openab_core::acp::SessionPool;
use openab_core::config::AgentConfig;
use openab_core::session_event::SessionStreamEvent;
use openab_core::session_snapshot::{SessionSnapshot, SessionStatus};
use openab_core::transcript::{ToolTranscriptUpdate, TranscriptSnapshot};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/fixtures/codeg-session-contract-v1.json"
));

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("Codeg contract fixture must be valid JSON")
}

fn endpoints(fixture: &Value) -> &Vec<Value> {
    fixture["endpoints"]
        .as_array()
        .expect("fixture endpoints must be an array")
}

fn endpoint<'a>(fixture: &'a Value, name: &str) -> &'a Value {
    endpoints(fixture)
        .iter()
        .find(|endpoint| endpoint["name"] == name)
        .unwrap_or_else(|| panic!("missing endpoint fixture: {name}"))
}

fn response<'a>(endpoint: &'a Value, name: &str) -> &'a Value {
    endpoint["responses"]
        .get(name)
        .unwrap_or_else(|| panic!("missing response fixture: {name}"))
}

fn assert_production_round_trip<T>(value: &Value, label: &str)
where
    T: DeserializeOwned + Serialize,
{
    let parsed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{label} must decode as a production type: {error}"));
    let serialized = serde_json::to_value(parsed)
        .unwrap_or_else(|error| panic!("{label} must serialize as a production type: {error}"));
    assert_eq!(serialized, *value, "{label} contains non-production fields");
}

fn without_dynamic_timestamps(mut value: Value) -> Value {
    fn visit(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                fields.remove("timestamp");
                fields.remove("created_at");
                fields.remove("updated_at");
                for value in fields.values_mut() {
                    visit(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value);
                }
            }
            _ => {}
        }
    }

    visit(&mut value);
    value
}

#[test]
fn declares_exactly_the_frozen_seven_endpoints() {
    let fixture = fixture();
    assert_eq!(fixture["contract"], "codeg-openab-session");
    assert_eq!(fixture["version"], 1);

    let expected = [
        ("list_sessions", "GET", "/api/v1/sessions"),
        ("create_session", "POST", "/api/v1/sessions"),
        ("get_session", "GET", "/api/v1/sessions/{session_id}"),
        (
            "get_transcript",
            "GET",
            "/api/v1/sessions/{session_id}/transcript",
        ),
        ("stream_session_events", "GET", "/api/v1/sessions/events"),
        (
            "send_message",
            "POST",
            "/api/v1/sessions/{session_id}/messages",
        ),
        (
            "cancel_session",
            "POST",
            "/api/v1/sessions/{session_id}/cancel",
        ),
    ];

    assert_eq!(endpoints(&fixture).len(), expected.len());
    for (name, method, path) in expected {
        let endpoint = endpoint(&fixture, name);
        assert_eq!(endpoint["method"], method, "method for {name}");
        assert_eq!(endpoint["path"], path, "path for {name}");
    }
}

#[test]
fn freezes_bearer_auth_and_opaque_session_id_rules() {
    let fixture = fixture();
    assert_eq!(fixture["session_id"], "admin:fixture-session");
    assert_eq!(fixture["encoded_session_id"], "admin%3Afixture-session");
    assert_eq!(fixture["auth"]["header"], "Authorization");
    assert_eq!(fixture["auth"]["scheme"], "Bearer");
    assert_eq!(fixture["auth"]["query_token"], false);

    for endpoint in endpoints(&fixture) {
        let headers = endpoint["request"]["headers"]
            .as_object()
            .expect("request headers must be an object");
        assert!(headers.contains_key("Authorization"), "missing auth header");
        assert!(!headers
            .keys()
            .any(|key| key.contains("token=") || key == "token"));
    }
}

#[test]
fn existing_session_shapes_are_represented_without_a_new_model() {
    let fixture = fixture();
    let snapshot = &fixture["snapshot"];
    assert_production_round_trip::<SessionSnapshot>(snapshot, "shared snapshot");

    let list = &endpoint(&fixture, "list_sessions")["response"]["body"];
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["session_id"], snapshot["session_id"]);
    assert_eq!(list[0]["title"], "请检查当前变更并运行测试");
    let mut list_snapshot = list[0].clone();
    list_snapshot
        .as_object_mut()
        .expect("list item must be an object")
        .remove("title");
    assert_production_round_trip::<SessionSnapshot>(&list_snapshot, "list snapshot");

    let created = &endpoint(&fixture, "create_session")["response"]["body"];
    assert_production_round_trip::<SessionSnapshot>(created, "created snapshot");

    let detail = &response(endpoint(&fixture, "get_session"), "success")["body"];
    assert_eq!(detail["session_id"], snapshot["session_id"]);
    assert_eq!(detail["status"], "idle");
    assert_production_round_trip::<SessionSnapshot>(detail, "detail snapshot");

    let transcript = &endpoint(&fixture, "get_transcript")["response"]["body"];
    assert_production_round_trip::<TranscriptSnapshot>(transcript, "transcript snapshot");
}

#[test]
fn freezes_message_request_ack_and_business_errors() {
    let fixture = fixture();
    let endpoint = endpoint(&fixture, "send_message");
    assert_eq!(
        endpoint["request"]["body"]
            .as_object()
            .map(|body| body.len()),
        Some(1)
    );
    assert_eq!(
        endpoint["request"]["body"]["text"],
        "请检查当前变更并运行测试"
    );
    assert_eq!(endpoint["http_response_contains_final_text"], false);

    let accepted = response(endpoint, "accepted");
    assert_eq!(accepted["status"], 202);
    assert_eq!(accepted["body"]["accepted"], true);
    assert_eq!(accepted["body"]["session_id"], fixture["session_id"]);
    assert!(accepted["body"].get("text").is_none());
    assert!(accepted["body"].get("content").is_none());

    for (name, status, error) in [
        ("unauthorized", 401, "invalid or missing admin token"),
        ("auth_not_configured", 503, "admin token is not configured"),
        ("empty_text", 400, "text is required"),
        ("missing_session", 404, "session not found"),
        ("busy", 409, "session is busy"),
        ("pre_accept_failure", 500, "failed to start session turn"),
    ] {
        let response = response(endpoint, name);
        assert_eq!(response["status"], status, "status for {name}");
        assert_eq!(response["body"]["error"], error, "error for {name}");
    }

    let agent_error = response(endpoint, "agent_error_after_accept");
    assert_eq!(agent_error["status"], 202);
    assert_eq!(agent_error["sse_result"]["event"], "error");
    assert_production_round_trip::<SessionStreamEvent>(
        &agent_error["sse_result"]["data"],
        "post-accept agent error",
    );
    assert_eq!(
        agent_error["sse_result"]["data"]["snapshot"]["status"],
        "error"
    );
    assert_eq!(
        agent_error["sse_result"]["data"]["snapshot"]["last_error"],
        "agent turn failed"
    );
}

#[test]
fn freezes_idempotent_cancel_and_best_effort_semantics() {
    let fixture = fixture();
    let endpoint = endpoint(&fixture, "cancel_session");
    assert!(endpoint["request"]["body"].is_null());
    assert_eq!(endpoint["idempotent_for_idle_session"], true);
    assert_eq!(endpoint["uses_acp_method"], "session/cancel");
    assert_eq!(endpoint["process_kill_guaranteed"], false);

    for name in ["accepted_running", "accepted_idle"] {
        let response = response(endpoint, name);
        assert_eq!(response["status"], 204, "status for {name}");
        assert!(response["body"].is_null(), "body for {name}");
        assert_eq!(
            response["sse_result"]["dedicated_cancel_event"], false,
            "dedicated cancel event for {name}"
        );
        assert_eq!(
            response["sse_result"]["guaranteed_events"]
                .as_array()
                .map(Vec::len),
            Some(0),
            "guaranteed events for {name}"
        );
    }

    let running_follow_up =
        &response(endpoint, "accepted_running")["sse_result"]["possible_follow_up"];
    assert_eq!(running_follow_up.as_array().map(Vec::len), Some(1));
    assert_eq!(running_follow_up[0]["event"], "status_changed");
    assert_production_round_trip::<SessionStreamEvent>(
        &running_follow_up[0]["data"],
        "cancel follow-up status",
    );

    let idle_follow_up = &response(endpoint, "accepted_idle")["sse_result"]["possible_follow_up"];
    assert_eq!(idle_follow_up.as_array().map(Vec::len), Some(0));

    for (name, status, error) in [
        ("missing_session", 404, "session not found"),
        ("send_failure", 500, "failed to cancel session"),
    ] {
        let response = response(endpoint, name);
        assert_eq!(response["status"], status, "status for {name}");
        assert_eq!(response["body"]["error"], error, "error for {name}");
    }
}

#[test]
fn freezes_sse_cursor_event_and_transcript_shapes() {
    let fixture = fixture();
    let stream = endpoint(&fixture, "stream_session_events");
    assert_eq!(stream["response"]["status"], 200);
    assert_eq!(stream["response"]["content_type"], "text/event-stream");
    assert_eq!(
        stream["request"]["headers"]["Last-Event-ID"],
        "fixture-generation:2"
    );

    let events = stream["response"]["events"]
        .as_array()
        .expect("SSE fixture events must be an array");
    let mut sequences = Vec::new();
    let mut entry_ids = HashSet::new();
    let mut saw_user = false;
    let mut saw_assistant = false;
    let mut saw_thinking = false;
    let mut saw_tool = false;

    for event in events {
        let id = event["id"].as_str().expect("SSE event id");
        assert!(id.starts_with("fixture-generation:"));
        let sequence = id
            .strip_prefix("fixture-generation:")
            .expect("qualified SSE id")
            .parse::<u64>()
            .expect("numeric SSE sequence");
        sequences.push(sequence);

        let event_name = event["event"].as_str().expect("SSE event name");
        let data = &event["data"];
        assert_eq!(data["sequence"], sequence);
        assert_production_round_trip::<SessionStreamEvent>(data, "SSE event data");
        if event_name == "transcript" {
            assert_eq!(data["session_id"], fixture["session_id"]);
            let entry = &data["entry"];
            let entry_id = entry["entry_id"].as_str().expect("entry id");
            entry_ids.insert(entry_id.to_string());
            match entry["role"].as_str() {
                Some("user") => saw_user = true,
                Some("assistant") if entry["status"] == "thinking" => saw_thinking = true,
                Some("assistant") => saw_assistant = true,
                Some("tool") => saw_tool = true,
                role => panic!("unexpected fixture transcript role: {role:?}"),
            }
        } else {
            assert_eq!(data["event"], event_name);
            assert!(data["snapshot"]["session_id"].is_string());
        }
    }

    assert_eq!(sequences, (3..=11).collect::<Vec<_>>());
    assert!(saw_user && saw_assistant && saw_thinking && saw_tool);
    assert!(entry_ids.contains("entry-1"));
    assert!(entry_ids.contains("entry-2"));
    assert!(entry_ids.contains("entry-3"));
    assert!(entry_ids.contains("entry-4"));
    assert!(entry_ids.contains("entry-5"));
}

#[tokio::test]
async fn transcript_and_sse_trace_are_emitted_by_current_production_stores() {
    let fixture = fixture();
    let session_id = fixture["session_id"].as_str().expect("fixture session id");
    let snapshot: SessionSnapshot = serde_json::from_value(fixture["snapshot"].clone())
        .expect("fixture snapshot must use the production type");
    let pool = SessionPool::new(AgentConfig::default(), 2, 120, HashMap::new());

    // The Last-Event-ID in the fixture starts after these two lifecycle events.
    pool.seed_session_snapshot_for_test(snapshot).await;
    pool.mark_session_status(session_id, SessionStatus::Running)
        .await;

    let transcripts = pool.transcript_store();
    transcripts.record_user_text(session_id, "请检查当前变更并运行测试");
    transcripts.append_assistant_text(session_id, "我会先检查变更，然后运行测试。");
    transcripts.append_thinking(session_id, "正在检查测试配置。");

    // This is the ordering used by record_acp_event_transcript for ToolStart.
    transcripts
        .finish_assistant_turn(session_id)
        .expect("active assistant entry");
    transcripts.upsert_tool_call(
        session_id,
        ToolTranscriptUpdate {
            tool_call_id: "tool-1".into(),
            title: "运行测试".into(),
            status: Some("running".into()),
            completed: false,
            payload: json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "运行测试",
                "rawInput": {"command": "cargo test"}
            }),
        },
    );
    transcripts.upsert_tool_call(
        session_id,
        ToolTranscriptUpdate {
            tool_call_id: "tool-1".into(),
            title: String::new(),
            status: Some("completed".into()),
            completed: true,
            payload: json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-1",
                "content": [{"type": "text", "text": "测试通过"}]
            }),
        },
    );
    transcripts.append_assistant_text(session_id, "检查完成，测试通过。");
    transcripts
        .finish_assistant_turn(session_id)
        .expect("post-tool assistant entry");
    pool.mark_session_status(session_id, SessionStatus::Idle)
        .await;

    assert_eq!(
        transcripts.session_title(session_id).as_deref(),
        Some("请检查当前变更并运行测试")
    );

    let mut actual_snapshot =
        serde_json::to_value(transcripts.snapshot(session_id, Some(1))).unwrap();
    actual_snapshot["stream_generation"] = json!("fixture-generation");
    let expected_snapshot = endpoint(&fixture, "get_transcript")["response"]["body"].clone();
    assert_eq!(
        without_dynamic_timestamps(actual_snapshot),
        without_dynamic_timestamps(expected_snapshot),
        "incremental transcript fixture must include every retained mutation"
    );

    let replay = pool.session_stream_bus().replay_after(2);
    assert!(!replay.overflowed);
    assert_eq!(replay.oldest_sequence, Some(1));
    assert_eq!(replay.next_sequence, 12);
    let actual_events = replay
        .events
        .into_iter()
        .map(|event| {
            let sequence = event.sequence();
            let event_name = event.as_sse_event();
            json!({
                "id": format!("fixture-generation:{sequence}"),
                "event": event_name,
                "data": event
            })
        })
        .collect::<Vec<_>>();
    let expected_events = endpoint(&fixture, "stream_session_events")["response"]["events"].clone();
    assert_eq!(
        without_dynamic_timestamps(Value::Array(actual_events)),
        without_dynamic_timestamps(expected_events),
        "SSE fixture must be an exact normalized production replay"
    );
}

#[test]
fn freezes_cursor_reset_history_gap_and_receiver_lag_recovery_events() {
    let fixture = fixture();
    let events = fixture["recovery_events"]
        .as_array()
        .expect("recovery events must be an array");
    assert_eq!(events.len(), 3);

    let names = events
        .iter()
        .filter_map(|event| event["event"].as_str())
        .collect::<HashSet<_>>();
    assert!(names.contains("cursor_reset"));
    assert!(names.contains("error"));

    let cursor_reset = events
        .iter()
        .find(|event| event["event"] == "cursor_reset")
        .expect("cursor reset fixture");
    assert_eq!(
        cursor_reset["data"]["error"],
        "event cursor generation changed"
    );
    assert_eq!(
        cursor_reset["data"]["action"],
        "refetch /api/v1/sessions before continuing the stream"
    );

    let history_gap = events
        .iter()
        .find(|event| event["data"]["error"] == "event history unavailable")
        .expect("history gap fixture");
    assert!(history_gap.get("id").is_none());
    assert_eq!(history_gap["data"]["oldest_sequence"], 5);
    assert_eq!(history_gap["data"]["next_sequence"], 9);

    let lagged = events
        .iter()
        .find(|event| event["data"]["error"] == "event stream lagged")
        .expect("receiver lag fixture");
    assert!(lagged.get("id").is_none());
    assert_eq!(lagged["data"]["skipped"], 2);
}
