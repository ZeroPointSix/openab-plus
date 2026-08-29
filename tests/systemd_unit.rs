//! Assertions for the packaging systemd unit template (ZER-869).
//! No root / systemctl required — reads the checked-in unit file.

const UNIT: &str = include_str!("../packaging/systemd/openab.service");
const ENV_EXAMPLE: &str = include_str!("../packaging/systemd/openab.env.example");
const SYSTEMD_DOC: &str = include_str!("../docs/systemd.md");
const ROOT_CARGO: &str = include_str!("../Cargo.toml");

#[test]
fn unit_has_required_directives() {
    assert!(
        UNIT.contains("KillMode=control-group"),
        "KillMode=control-group required so agent cgroup children are reaped"
    );
    assert!(
        UNIT.contains("Environment=PATH="),
        "explicit Environment=PATH= required (Factory sgp-001 gap)"
    );
    assert!(
        UNIT.contains("Environment=HOME="),
        "explicit Environment=HOME= required (real login home, D2)"
    );
    assert!(
        UNIT.contains("OPENAB_HOME="),
        "OPENAB_HOME must be set for externalized state"
    );
    assert!(
        UNIT.contains("OPENAB_WORK_DIR="),
        "OPENAB_WORK_DIR must be set"
    );
    assert!(
        UNIT.contains("OPENAB_LOG_DIR="),
        "OPENAB_LOG_DIR must be set"
    );
    assert!(
        UNIT.contains("Restart=always"),
        "Restart=always required"
    );
    assert!(
        UNIT.contains("Type=simple"),
        "Type=simple required"
    );
    assert!(
        UNIT.contains("EnvironmentFile=-/etc/openab/openab.env"),
        "optional EnvironmentFile with leading - required"
    );
    assert!(
        UNIT.contains("ExecStart=/usr/local/bin/openab run -c /etc/openab/config.toml"),
        "ExecStart must invoke openab run with config path"
    );
    assert!(
        UNIT.contains("OPENAB_AUTO_UPDATE=false"),
        "OPENAB_AUTO_UPDATE=false placeholder required"
    );
    assert!(
        UNIT.contains("TimeoutStopSec="),
        "TimeoutStopSec required with KillMode=control-group"
    );
}

#[test]
fn unit_has_no_inbound_socket_activation() {
    for forbidden in ["Accept=", "ListenStream=", "ListenDatagram=", "ListenFIFO="] {
        assert!(
            !UNIT.lines().any(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with('#') && trimmed.contains(forbidden)
            }),
            "unit must not open inbound sockets ({forbidden})"
        );
    }
    let exec_start = UNIT
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with("ExecStart=") && !line.starts_with("ExecStartPost"))
        .expect("ExecStart= line");
    assert!(
        !exec_start.split_whitespace().any(|tok| tok == "-p"),
        "ExecStart must not pass -p to open a listen port: {exec_start}"
    );
}

#[test]
fn env_example_has_names_only_no_live_tokens() {
    assert!(ENV_EXAMPLE.contains("DISCORD_BOT_TOKEN="));
    assert!(ENV_EXAMPLE.contains("SLACK_BOT_TOKEN="));
    assert!(ENV_EXAMPLE.contains("OPENAB_AUTO_UPDATE="));
    for line in ENV_EXAMPLE.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Active assignments must not embed secret-looking values.
        if let Some((_, value)) = trimmed.split_once('=') {
            assert!(
                value.is_empty()
                    || value.starts_with("${")
                    || value.eq_ignore_ascii_case("false")
                    || value.eq_ignore_ascii_case("true"),
                "env example must not ship real token values: {trimmed}"
            );
        }
    }
}

#[test]
fn daemon_feature_documented_and_aligned() {
    assert!(
        ROOT_CARGO.contains("daemon = [\"discord\", \"slack\"]")
            || ROOT_CARGO.contains("daemon = [\"discord\", \"slack\","),
        "root Cargo.toml must define opt-in daemon = discord+slack"
    );
    assert!(
        !ROOT_CARGO
            .lines()
            .find(|l| l.trim_start().starts_with("default ="))
            .expect("default features line")
            .contains("daemon"),
        "daemon must not be folded into default features"
    );
    assert!(
        SYSTEMD_DOC.contains("cargo build --release --no-default-features --features daemon"),
        "docs/systemd.md must document the daemon build command"
    );
    for excluded in [
        "agentcore",
        "pre-seed",
        "config-s3",
        "secrets-aws",
        "filestore",
        "unified",
        "telegram",
        "line",
        "feishu",
        "googlechat",
        "wecom",
        "teams",
        "acp",
    ] {
        assert!(
            SYSTEMD_DOC.contains(excluded),
            "docs/systemd.md should call out excluded feature `{excluded}`"
        );
    }
}
