//! 宣告式 adapter 的兩項安全宣告：
//! 1. `limits:`＝**裝置安全上限**，必須進 manifest，Policy Governor 的
//!    min(AI 請求, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算)
//!    才有「裝置」那一項（只靠韌體自己 clamp 保護不了不 clamp 的裝置）。
//! 2. 明文憑證（pairingCode／password 沒寫成 secret://）要留下可稽核的警告。
//!
//! 全部是純 spec／manifest 層的檢查，不碰任何真硬體。

use interaction_adapter_declarative::{build, credential_warnings, parse_spec};

fn http_actuator_spec(limits: &str) -> String {
    format!(
        r#"
schemaVersion: "1.0"
id: desk-lamp
capabilities:
  - kind: actuator
    id: glow
    channel: light
    transport: http
    timeoutMs: 500
    request: {{ method: POST, url: "http://127.0.0.1:1/set", body: {{}} }}
{limits}
"#
    )
}

/// YAML 宣告的裝置上限必須原封不動出現在 manifest：那是 Policy 夾值時
/// 讀的 `actuator.limits.max_magnitude`（決策記成 `magnitude.device`）。
#[test]
fn declared_device_limits_reach_the_actuator_manifest() {
    let yaml = http_actuator_spec(
        "    limits:\n      maxMagnitude: 0.6\n      maxDurationMs: 2000\n      maxPerHour: 10",
    );
    let spec = parse_spec(&yaml).expect("spec parses");
    let built = build(&spec, None).expect("build");
    let manifest = built.actuators[0].manifest();
    assert_eq!(
        manifest.limits.max_magnitude,
        Some(0.6),
        "裝置安全上限必須進 manifest：{:?}",
        manifest.limits
    );
    assert_eq!(manifest.limits.max_duration_ms, Some(2_000));
    assert_eq!(manifest.limits.max_per_hour, Some(10));
}

/// 沒宣告 limits 就是「這台裝置沒有自報上限」——欄位維持 None，
/// 不得偷偷編一個出來（假的上限比沒有上限更危險）。
#[test]
fn a_spec_without_limits_declares_no_ceiling() {
    let spec = parse_spec(&http_actuator_spec("")).expect("spec parses");
    let built = build(&spec, None).expect("build");
    let manifest = built.actuators[0].manifest();
    assert_eq!(manifest.limits.max_magnitude, None);
    assert_eq!(manifest.limits.max_duration_ms, None);
}

/// 壞的上限一律誠實拒絕：範圍外／NaN 讓 min() 行為無法預期，
/// 0 會讓動器變成永遠拒絕（看起來像壞掉，不像被限制）。
#[test]
fn nonsensical_limits_are_refused_at_parse_time() {
    for bad in [
        "    limits:\n      maxMagnitude: 1.5",
        "    limits:\n      maxMagnitude: -0.1",
        "    limits:\n      maxDurationMs: 0",
        "    limits:\n      maxPerHour: 0",
    ] {
        let err = parse_spec(&http_actuator_spec(bad)).expect_err(&format!("must reject {bad:?}"));
        assert!(err.contains("glow"), "錯誤要指出是哪個能力：{err}");
    }
}

/// link 傳輸（serial）的動器同樣要把裝置上限帶進 manifest。
#[cfg(feature = "transport-serial")]
#[test]
fn link_actuators_carry_device_limits_too() {
    let yaml = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: serial
    command: { name: "vibe.pulse", params: { strength: "{{magnitude}}" } }
    limits:
      maxMagnitude: 0.6
      maxDurationMs: 3000
    serial:
      port: "/nonexistent/limits-probe"
      expectedDeviceId: "esp32-desk01"
"#;
    let spec = parse_spec(yaml).expect("spec parses");
    let built = build(&spec, None).expect("build");
    let manifest = built.actuators[0].manifest();
    assert_eq!(manifest.limits.max_magnitude, Some(0.6));
    assert_eq!(manifest.limits.max_duration_ms, Some(3_000));
    // 連線一定要關掉，測試不留背景重連執行緒。
    for link in &built.links {
        link.shutdown();
    }
}

/// 明文憑證：不阻擋（既有 spec 不能一升級就壞掉），但必須留下可稽核的
/// 警告，而且訊息只點名能力與欄位——**絕不回顯憑證值**。
#[cfg(feature = "transport-mqtt")]
#[tokio::test]
async fn plaintext_credentials_are_warned_and_never_echoed() {
    let yaml = r#"
schemaVersion: "1.0"
id: esp32-mqtt
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: mqtt
    command: { name: "vibe.pulse", params: {} }
    mqtt:
      brokerHost: "127.0.0.1"
      brokerPort: 1
      topicPrefix: "companion/plain"
      expectedDeviceId: "esp32-desk01"
      pairingCode: "9927"
      password: "hunter2"
"#;
    let spec = parse_spec(yaml).expect("spec parses");
    let warnings = credential_warnings(&spec);
    assert_eq!(
        warnings.len(),
        2,
        "pairingCode 與 password 各一則：{warnings:?}"
    );
    let joined = warnings.join("\n");
    assert!(joined.contains("vibe"), "{joined}");
    assert!(joined.contains("pairingCode"), "{joined}");
    assert!(joined.contains("password"), "{joined}");
    assert!(joined.contains("secret://"), "要說明怎麼改：{joined}");
    assert!(!joined.contains("9927"), "警告不得回顯憑證值：{joined}");
    assert!(!joined.contains("hunter2"), "警告不得回顯憑證值：{joined}");

    // 同一份警告要掛在 build 結果上（呼叫端才有東西可顯示／稽核）。
    let built = build(&spec, None).expect("build");
    assert_eq!(built.warnings, warnings);
    for link in &built.links {
        link.shutdown();
    }
}

/// 用 secret:// 參照就沒有警告（這正是被建議的寫法）。
#[cfg(feature = "transport-mqtt")]
#[test]
fn secret_references_produce_no_warning() {
    let yaml = r#"
schemaVersion: "1.0"
id: esp32-mqtt-secret
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: mqtt
    command: { name: "vibe.pulse", params: {} }
    mqtt:
      brokerHost: "127.0.0.1"
      brokerPort: 1
      topicPrefix: "companion/secret"
      expectedDeviceId: "esp32-desk01"
      pairingCode: "secret://esp32-pairing"
      password: "secret://mqtt-password"
"#;
    let spec = parse_spec(yaml).expect("spec parses");
    assert!(credential_warnings(&spec).is_empty());
}
