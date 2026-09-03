//! Executable shape checks for the Codeg/OpenAB session contract fixture.
//!
//! The two new write routes are intentionally not implemented by this issue.
//! This test makes the frozen fixture fail loudly if its endpoint, error, or
//! event shape drifts before the implementation PR consumes it.

use serde_json::Value;
use std::collections::HashSet;

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
    for field in [
        "session_id",
        "agent",
        "source",
        "workdir",
        "status",
        "created_at",
        "updated_at",
    ] {
        assert!(snapshot.get(field).is_some(), "snapshot missing {field}");
    }

    let list = &endpoint(&fixture, "list_sessions")["response"]["body"];
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["session_id"], snapshot["session_id"]);
    assert_eq!(list[0]["title"], "请检查当前变更并运行测试");

    let detail = &response(endpoint(&fixture, "get_session"), "success")["body"];
    assert_eq!(detail["session_id"], snapshot["session_id"]);
    assert_eq!(detail["status"], "idle");

    let transcript = &endpoint(&fixture, "get_transcript")["response"]["body"];
    assert_eq!(transcript["session_id"], snapshot["session_id"]);
    assert_eq!(transcript["overflowed"], false);
    assert_eq!(transcript["stream_generation"], "fixture-generation");
    assert_eq!(transcript["stream_next_sequence"], 7);
    assert!(transcript["entries"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| entry["role"] == "assistant")
            && entries.iter().any(|entry| entry["role"] == "tool")
    }));
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
    ] {
        let response = response(endpoint, name);
        assert_eq!(response["status"], status, "status for {name}");
        assert_eq!(response["body"]["error"], error, "error for {name}");
    }
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
        assert_eq!(response["status"], 200, "status for {name}");
        assert_eq!(response["body"]["accepted"], true);
        assert_eq!(response["body"]["session_id"], fixture["session_id"]);
    }

    let missing = response(endpoint, "missing_session");
    assert_eq!(missing["status"], 404);
    assert_eq!(missing["body"]["error"], "session not found");
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

    assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(saw_user && saw_assistant && saw_thinking && saw_tool);
    assert!(entry_ids.contains("entry-1"));
    assert!(entry_ids.contains("entry-2"));
    assert!(entry_ids.contains("entry-3"));
    assert!(entry_ids.contains("entry-4"));
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
