//! Fixture coverage tests: the checked-in integration configs must stay
//! syntactically valid, cover every version 1 lifecycle event, and never
//! interpolate hook payloads into the emitter command line.

use std::path::PathBuf;

fn fixture(relative: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Payload tokens that must never appear in a hook command: interpolating the
/// hook document into argv would ship prompt or tool content to the overlay.
const FORBIDDEN_PAYLOAD_TOKENS: [&str; 6] = [
    "${prompt}",
    "$prompt",
    "last-assistant-message",
    "input-messages",
    "tool_input",
    "tool_output",
];

#[test]
fn codex_fixture_covers_required_events_without_content_arguments() {
    let text = std::fs::read_to_string(fixture("integrations/codex/hooks.toml")).unwrap();
    let _: toml::Value = toml::from_str(&text).unwrap();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "Stop",
        "SessionEnd",
    ] {
        assert!(text.contains(event));
    }
    for forbidden in FORBIDDEN_PAYLOAD_TOKENS {
        assert!(
            !text.contains(forbidden),
            "forbidden payload interpolation: {forbidden}"
        );
    }
}

#[test]
fn opencode_plugin_maps_the_lifecycle_events_with_the_opencode_source() {
    let text =
        std::fs::read_to_string(fixture("integrations/opencode/termielle.plugin.ts")).unwrap();

    for wire in [
        "prompt_submitted",
        "needs_input",
        "turn_completed",
        "turn_failed",
    ] {
        assert!(
            text.contains(&format!("emit(\"{wire}\"")),
            "missing {wire} mapping"
        );
    }

    assert!(text.contains("--source opencode"));
    assert!(
        !text.contains("claude") && !text.contains("codex"),
        "opencode plugin must not carry another source"
    );
    assert!(
        text.contains("--input argv"),
        "the plugin must pass its session document through argv"
    );
    assert!(
        !text.contains("session.next.text") && !text.contains("tool.input"),
        "the plugin must not map content-bearing events"
    );

    for forbidden in FORBIDDEN_PAYLOAD_TOKENS {
        assert!(
            !text.contains(forbidden),
            "forbidden payload interpolation: {forbidden}"
        );
    }
}

#[test]
fn claude_fixture_is_json_and_maps_stop_failure() {
    let text =
        std::fs::read_to_string(fixture("integrations/claude/settings.fragment.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(value["hooks"]["StopFailure"].is_array());
    assert!(text.contains("--event turn_failed"));
}

#[test]
fn codex_fixture_uses_the_codex_source_and_the_argv_notify_document() {
    let text = std::fs::read_to_string(fixture("integrations/codex/hooks.toml")).unwrap();
    assert!(text.contains("--source codex"));
    assert!(
        !text.contains("claude"),
        "codex hooks must not carry a claude source"
    );
    assert!(
        !text.contains("turn_failed"),
        "codex exposes no failure mapping"
    );
    assert!(
        !text.contains("StopFailure"),
        "codex hooks have no StopFailure event"
    );
    // The legacy notify fallback receives the agent-turn-complete document as
    // its argv JSON, which the emitter reads via --input argv.
    assert!(text.contains("--event turn_completed"));
    assert!(text.contains("--input argv"));
    assert!(
        text.contains("timeout = 1"),
        "hooks must use a one-second timeout"
    );
}

#[test]
fn claude_fixture_covers_all_six_mapped_events_without_payload_tokens() {
    let text =
        std::fs::read_to_string(fixture("integrations/claude/settings.fragment.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();

    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "Stop",
        "StopFailure",
        "SessionEnd",
    ];
    for event in events {
        assert!(
            value["hooks"][event].is_array(),
            "{event} hooks must be an array"
        );
    }

    for wire in [
        "session_started",
        "prompt_submitted",
        "needs_input",
        "turn_completed",
        "turn_failed",
        "session_ended",
    ] {
        assert!(
            text.contains(&format!("--event {wire}")),
            "missing --event {wire}"
        );
    }

    assert!(text.contains("--source claude"));
    assert!(
        !text.contains("codex"),
        "claude hooks must not carry a codex source"
    );
    assert!(
        text.contains("\"timeout\": 1"),
        "hooks must use a one-second timeout"
    );

    for forbidden in FORBIDDEN_PAYLOAD_TOKENS {
        assert!(
            !text.contains(forbidden),
            "forbidden payload interpolation: {forbidden}"
        );
    }
}
