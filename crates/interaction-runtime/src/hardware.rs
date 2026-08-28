//! Cross-platform, metadata-only hardware discovery.
//!
//! Scanning never opens a camera/microphone, starts HID capture, powers a BLE
//! radio, browses mDNS, or connects to a device. Unsupported/permission-gated
//! categories are explicit rows, not fake devices.

use crate::runtime::Runtime;
use interaction_core::{
    DiscoveredCapability, DiscoveredCapabilityKind as Kind, DiscoveredHardware,
    HardwareAvailability as Availability, HardwareClass as Class, HardwareDiscoveryAdapter,
    HardwareScanReport,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Stdio;

pub struct SystemPresentationDiscovery;

#[async_trait::async_trait]
impl HardwareDiscoveryAdapter for SystemPresentationDiscovery {
    fn id(&self) -> &'static str {
        "system-presentation"
    }

    async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String> {
        let platform = std::env::consts::OS;
        Ok(vec![
            DiscoveredHardware {
                class: Class::Display,
                display_name: "作業系統顯示呈現".into(),
                stable_id: Some(format!("system:{platform}:display-presentation")),
                identity_basis: "作業系統能力（不是特定螢幕序號）".into(),
                availability: Availability::Available,
                permission_requirements: vec![],
                capabilities: vec![cap(
                    "system.display.present",
                    Kind::Actuator,
                    "只在本應用視窗內呈現內容",
                    false,
                    true,
                    false,
                )],
                source_adapter: self.id().into(),
                detail: "只確認應用可建立視窗；未宣稱列出所有實體螢幕".into(),
            },
            DiscoveredHardware {
                class: Class::SystemNotification,
                display_name: "系統通知".into(),
                stable_id: Some(format!("system:{platform}:notifications")),
                identity_basis: "作業系統通知服務".into(),
                availability: Availability::Available,
                permission_requirements: vec!["作業系統通知權限可能尚未授予".into()],
                capabilities: vec![cap(
                    "system.notification.show",
                    Kind::Actuator,
                    "顯示一則系統通知",
                    false,
                    true,
                    false,
                )],
                source_adapter: self.id().into(),
                detail: "可呼叫通知服務；實際送達仍以 Action Receipt 為準".into(),
            },
            DiscoveredHardware {
                class: Class::AudioOutput,
                display_name: "系統音訊輸出".into(),
                stable_id: Some(format!("system:{platform}:audio-output")),
                identity_basis: "作業系統預設輸出路由（不是特定喇叭身分）".into(),
                availability: Availability::Unknown,
                permission_requirements: vec![],
                capabilities: vec![cap(
                    "system.audio.play",
                    Kind::Actuator,
                    "播放已登記音效；預設關閉",
                    false,
                    true,
                    true,
                )],
                source_adapter: self.id().into(),
                detail: "沒有打開音訊裝置，因此無法在掃描時確認目前路由是否可用".into(),
            },
        ])
    }
}

/// 只查看穩定 OS symlink 名稱；不開啟任何 device node。
pub struct StableDeviceLinkDiscovery;

#[async_trait::async_trait]
impl HardwareDiscoveryAdapter for StableDeviceLinkDiscovery {
    fn id(&self) -> &'static str {
        "stable-device-links"
    }

    async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String> {
        tokio::task::spawn_blocking(|| {
            let mut found = Vec::new();
            #[cfg(target_os = "linux")]
            {
                collect_linux_by_id(
                    "/dev/v4l/by-id",
                    Class::Camera,
                    "camera",
                    &mut found,
                    vec![cap(
                        "camera.frames",
                        Kind::Receptor,
                        "攝影機影像；掃描不會啟動",
                        true,
                        false,
                        true,
                    )],
                );
                collect_linux_by_id(
                    "/dev/serial/by-id",
                    Class::UsbSerial,
                    "usb-serial",
                    &mut found,
                    vec![
                        cap(
                            "serial.read",
                            Kind::Receptor,
                            "裝置宣告的 serial 輸入",
                            true,
                            false,
                            true,
                        ),
                        cap(
                            "serial.write",
                            Kind::Actuator,
                            "裝置宣告的 serial 輸出",
                            false,
                            true,
                            true,
                        ),
                    ],
                );
                collect_linux_by_id(
                    "/dev/input/by-id",
                    Class::Keyboard,
                    "hid",
                    &mut found,
                    vec![cap(
                        "hid.semantic-input",
                        Kind::Receptor,
                        "經個別授權的語意輸入；不保存全域按鍵",
                        true,
                        false,
                        true,
                    )],
                );
            }
            #[cfg(target_os = "macos")]
            {
                // macOS /dev/cu.* 是會變動的連線路徑，不當永久身分。仍可
                // 誠實呈現候選，但 stableId 留空，不能直接配對／安裝。
                if let Ok(entries) = std::fs::read_dir("/dev") {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("cu.usbmodem") || name.starts_with("cu.usbserial") {
                            found.push(DiscoveredHardware {
                                class: Class::UsbSerial,
                                display_name: name,
                                stable_id: None,
                                identity_basis: "volatile /dev 路徑；不是永久身分".into(),
                                availability: Availability::Unknown,
                                permission_requirements: vec![
                                    "需要裝置序號或配對指紋後才能安裝".into()
                                ],
                                capabilities: vec![],
                                source_adapter: "stable-device-links".into(),
                                detail: "只看見 serial 路徑；掃描沒有開啟連線".into(),
                            });
                        }
                    }
                }
            }
            found
        })
        .await
        .map_err(|e| format!("device-link scan task failed: {e}"))
    }
}

/// macOS `system_profiler` metadata adapter.
///
/// The command reads registry metadata only. It does not open AVFoundation,
/// CoreAudio input streams, HID event taps, Bluetooth discovery, or device
/// nodes. Raw serial numbers/addresses are never returned as identifiers: the
/// runtime exposes a namespaced SHA-256 fingerprint instead.
pub struct MacSystemProfilerDiscovery;

#[async_trait::async_trait]
impl HardwareDiscoveryAdapter for MacSystemProfilerDiscovery {
    fn id(&self) -> &'static str {
        "macos-system-profiler"
    }

    async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String> {
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Vec::new())
        }
        #[cfg(target_os = "macos")]
        {
            let mut command = tokio::process::Command::new("/usr/sbin/system_profiler");
            command
                .args([
                    "-json",
                    "SPCameraDataType",
                    "SPAudioDataType",
                    "SPDisplaysDataType",
                    "SPUSBDataType",
                    "SPUSBHostDataType",
                    "SPBluetoothDataType",
                    "SPMIDIDataType",
                ])
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .kill_on_drop(true);
            let output = tokio::time::timeout(std::time::Duration::from_secs(12), command.output())
                .await
                .map_err(|_| "system_profiler metadata scan timed out after 12s".to_string())?
                .map_err(|e| format!("run system_profiler: {e}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "system_profiler exited {}: {}",
                    output.status,
                    stderr.trim().chars().take(240).collect::<String>()
                ));
            }
            let value: serde_json::Value = serde_json::from_slice(&output.stdout)
                .map_err(|e| format!("parse system_profiler JSON: {e}"))?;
            Ok(parse_macos_profiler(&value))
        }
    }
}

/// Human-owned declarative adapters are the only first-version source for
/// approved `.local`/ESP32 devices. This scans configuration metadata only;
/// it never browses mDNS, connects to an endpoint, or resolves a secret.
pub struct ApprovedLocalDeclarationDiscovery {
    pub home: PathBuf,
}

#[async_trait::async_trait]
impl HardwareDiscoveryAdapter for ApprovedLocalDeclarationDiscovery {
    fn id(&self) -> &'static str {
        "approved-local-declarations"
    }

    async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String> {
        let directory = self.home.join("config").join("adapters");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(vec![]);
        };
        let mut rows = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| matches!(value, "yaml" | "yml" | "json"))
            {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("讀取 {} 失敗：{error}", path.display()))?;
            let spec = interaction_adapter_declarative::parse_spec(&text)
                .map_err(|error| format!("{}：{error}", path.display()))?;
            // v0.5：link 傳輸（serial/mqtt/ble）沒有 request.url——指紋來源改用
            // 「有什麼就用什麼」：http url、serial 埠、mqtt broker/topic、ble 名稱。
            let fingerprint_source = spec
                .capabilities
                .iter()
                .filter_map(|capability| {
                    if let Some(request) = &capability.request {
                        return Some(request.url.clone());
                    }
                    if let Some(serial) = &capability.serial {
                        return Some(format!("serial:{}", serial.port));
                    }
                    if let Some(mqtt) = &capability.mqtt {
                        return Some(format!(
                            "mqtt:{}:{}/{}",
                            mqtt.broker_host, mqtt.broker_port, mqtt.topic_prefix
                        ));
                    }
                    capability
                        .ble
                        .as_ref()
                        .map(|ble| format!("ble:{}", ble.device_name))
                })
                .collect::<Vec<_>>();
            let joined = fingerprint_source.join("\0");
            let label = spec.display_name.clone().unwrap_or_else(|| spec.id.clone());
            let descriptor = format!("{} {}", spec.id, label).to_lowercase();
            let is_esp32 = descriptor.contains("esp32") || descriptor.contains("espressif");
            let is_mdns = fingerprint_source
                .iter()
                .any(|url| url.to_lowercase().contains(".local"));
            if !is_esp32 && !is_mdns {
                continue;
            }
            let class = if is_esp32 {
                Class::Esp32Declaration
            } else {
                Class::MdnsDevice
            };
            let capabilities = spec
                .capabilities
                .iter()
                .map(|declared| {
                    let (kind, read, write) = match declared.kind {
                        interaction_adapter_declarative::CapabilityKindSpec::Receptor => {
                            (Kind::Receptor, true, false)
                        }
                        interaction_adapter_declarative::CapabilityKindSpec::Actuator => {
                            (Kind::Actuator, false, true)
                        }
                    };
                    cap(
                        &declared.id,
                        kind,
                        declared
                            .description
                            .as_deref()
                            .unwrap_or("使用者宣告的本機裝置能力"),
                        read,
                        write,
                        true,
                    )
                })
                .collect();
            rows.push(DiscoveredHardware {
                class,
                display_name: label,
                stable_id: namespaced_fingerprint(
                    "declaration",
                    if is_esp32 { "esp32" } else { "mdns" },
                    &[&spec.id, &joined],
                ),
                identity_basis:
                    "使用者核准的 declarative adapter id/endpoint fingerprint（不顯示位址）".into(),
                availability: Availability::PermissionRequired,
                permission_requirements: vec!["仍須完成配對、逐能力使用授權與啟用".into()],
                capabilities,
                source_adapter: self.id().into(),
                detail: "只讀取已核准設定檔；未瀏覽 mDNS、未連線、未解析 secret://".into(),
            });
        }
        Ok(rows)
    }
}

#[cfg(any(target_os = "macos", test))]
fn stable_fingerprint(kind: &str, parts: &[&str]) -> Option<String> {
    namespaced_fingerprint("macos", kind, parts)
}

fn namespaced_fingerprint(namespace: &str, kind: &str, parts: &[&str]) -> Option<String> {
    if parts.iter().all(|part| part.trim().is_empty()) {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"adaptive-interaction:hardware-identity:v1\0");
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update(b"\0");
        hasher.update(part.trim().as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    Some(format!("{namespace}:{kind}:{}", &digest[..24]))
}

#[cfg(any(target_os = "macos", test))]
fn value_text<'a>(value: &'a serde_json::Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| match value {
            serde_json::Value::String(text) => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

#[cfg(any(target_os = "macos", test))]
fn value_nonzero(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).is_some_and(|v| match v {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or_default() > 0,
        serde_json::Value::String(s) => !s.is_empty() && s != "0",
        _ => false,
    })
}

#[cfg(any(target_os = "macos", test))]
fn nested_items<'a>(value: &'a serde_json::Value, key: &str) -> Vec<&'a serde_json::Value> {
    let mut out = Vec::new();
    let Some(items) = value.get(key).and_then(|v| v.as_array()) else {
        return out;
    };
    for item in items {
        collect_profiler_items(item, &mut out);
    }
    out
}

#[cfg(any(target_os = "macos", test))]
fn collect_profiler_items<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
    out.push(value);
    for child_key in ["_items", "spdisplays_ndrvs"] {
        if let Some(children) = value.get(child_key).and_then(|v| v.as_array()) {
            for child in children {
                collect_profiler_items(child, out);
            }
        }
    }
}

/// Convert the documented `system_profiler -json` shape into the platform-
/// neutral discovery contract. Kept deterministic so captured OS fixtures can
/// exercise the real adapter boundary without touching a sensor in tests.
#[cfg(any(target_os = "macos", test))]
fn parse_macos_profiler(root: &serde_json::Value) -> Vec<DiscoveredHardware> {
    let mut rows = Vec::new();

    for camera in root
        .get("SPCameraDataType")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let name = value_text(camera, &["_name", "spcamera_model-id"]);
        let unique = value_text(camera, &["spcamera_unique-id"]);
        let model = value_text(camera, &["spcamera_model-id"]);
        rows.push(DiscoveredHardware {
            class: Class::Camera,
            display_name: if name.is_empty() {
                "攝影機".into()
            } else {
                name.into()
            },
            stable_id: stable_fingerprint("camera", &[unique, model]),
            identity_basis: "system_profiler camera unique-id/model（只輸出雜湊）".into(),
            availability: Availability::PermissionRequired,
            permission_requirements: vec!["讀取影像前需個別攝影機使用授權".into()],
            capabilities: vec![cap(
                "camera.frames",
                Kind::Receptor,
                "攝影機影像；metadata 掃描不啟動影像串流",
                true,
                false,
                true,
            )],
            source_adapter: "macos-system-profiler".into(),
            detail: "已偵測到 OS 宣告的攝影機；未開啟、未擷取畫面".into(),
        });
    }

    for audio in nested_items(root, "SPAudioDataType") {
        let name = value_text(audio, &["_name"]);
        let manufacturer = value_text(audio, &["coreaudio_device_manufacturer"]);
        let transport = value_text(audio, &["coreaudio_device_transport"]);
        let identity = stable_fingerprint("audio", &[name, manufacturer, transport]);
        if value_nonzero(audio, "coreaudio_device_input") {
            for class in [Class::Microphone, Class::AudioInput] {
                rows.push(DiscoveredHardware {
                    class,
                    display_name: if name.is_empty() {
                        "音訊輸入".into()
                    } else {
                        name.into()
                    },
                    stable_id: identity.clone(),
                    identity_basis:
                        "CoreAudio name/manufacturer/transport fingerprint（只輸出雜湊）".into(),
                    availability: Availability::PermissionRequired,
                    permission_requirements: vec!["開始音訊輸入前需個別使用授權".into()],
                    capabilities: vec![cap(
                        if class == Class::Microphone {
                            "microphone.level"
                        } else {
                            "audio.input"
                        },
                        Kind::Receptor,
                        "音訊輸入；metadata 掃描不啟動或保存音訊",
                        true,
                        false,
                        true,
                    )],
                    source_adapter: "macos-system-profiler".into(),
                    detail: "已偵測到 CoreAudio 輸入宣告；未開啟輸入 stream".into(),
                });
            }
        }
        if value_nonzero(audio, "coreaudio_device_output") {
            rows.push(DiscoveredHardware {
                class: Class::AudioOutput,
                display_name: if name.is_empty() {
                    "音訊輸出".into()
                } else {
                    name.into()
                },
                stable_id: identity,
                identity_basis: "CoreAudio name/manufacturer/transport fingerprint（只輸出雜湊）"
                    .into(),
                availability: Availability::Available,
                permission_requirements: vec!["播放音效仍需個別啟用".into()],
                capabilities: vec![cap(
                    "audio.output.play",
                    Kind::Actuator,
                    "播放已登記且受上限約束的音效",
                    false,
                    true,
                    true,
                )],
                source_adapter: "macos-system-profiler".into(),
                detail: "已偵測到 CoreAudio 輸出宣告；掃描沒有播放音訊".into(),
            });
        }
    }

    for display in nested_items(root, "SPDisplaysDataType") {
        if display.get("_spdisplays_display-product-id").is_none() {
            continue;
        }
        let name = value_text(display, &["_name"]);
        let serial = value_text(display, &["_spdisplays_display-serial-number"]);
        let vendor = value_text(display, &["_spdisplays_display-vendor-id"]);
        let product = value_text(display, &["_spdisplays_display-product-id"]);
        rows.push(DiscoveredHardware {
            class: Class::Display,
            display_name: if name.is_empty() {
                "顯示器".into()
            } else {
                name.into()
            },
            stable_id: stable_fingerprint("display", &[serial, vendor, product]),
            identity_basis: "display serial/vendor/product fingerprint（只輸出雜湊）".into(),
            availability: Availability::Available,
            permission_requirements: vec![],
            capabilities: vec![cap(
                "display.present",
                Kind::Actuator,
                "在本應用視窗顯示內容",
                false,
                true,
                false,
            )],
            source_adapter: "macos-system-profiler".into(),
            detail: "已偵測顯示器中繼資料；未進行螢幕讀取或截圖".into(),
        });
    }

    for root_key in ["SPUSBDataType", "SPUSBHostDataType"] {
        for usb in nested_items(root, root_key) {
            let name = value_text(usb, &["_name"]);
            let lower = name.to_lowercase();
            let serial_like = ["serial", "uart", "modem", "arduino", "esp32"]
                .iter()
                .any(|needle| lower.contains(needle));
            let serial = value_text(usb, &["serial_num", "serial_number"]);
            let vendor = value_text(usb, &["vendor_id"]);
            let product = value_text(usb, &["product_id"]);
            if serial_like {
                rows.push(DiscoveredHardware {
                    class: Class::UsbSerial,
                    display_name: if name.is_empty() {
                        "USB Serial".into()
                    } else {
                        name.into()
                    },
                    stable_id: stable_fingerprint("usb-serial", &[serial, vendor, product]),
                    identity_basis: "USB serial/vendor/product fingerprint（只輸出雜湊）".into(),
                    availability: Availability::PermissionRequired,
                    permission_requirements: vec!["配對後須個別授權 serial read/write".into()],
                    capabilities: vec![
                        cap(
                            "serial.read",
                            Kind::Receptor,
                            "USB Serial 輸入",
                            true,
                            false,
                            true,
                        ),
                        cap(
                            "serial.write",
                            Kind::Actuator,
                            "USB Serial 輸出",
                            false,
                            true,
                            true,
                        ),
                    ],
                    source_adapter: "macos-system-profiler".into(),
                    detail: "只列舉 USB registry metadata；未開啟 serial port".into(),
                });
                continue;
            }

            let hid_class = if lower.contains("keyboard") {
                Some(Class::Keyboard)
            } else if lower.contains("trackpad") || lower.contains("touchpad") {
                Some(Class::Touchpad)
            } else if lower.contains("mouse") {
                Some(Class::Mouse)
            } else if ["tablet", "wacom", "pen"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                Some(Class::Tablet)
            } else if ["game", "controller", "joystick", "dualshock", "dualsense"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                Some(Class::GameController)
            } else {
                None
            };
            if let Some(class) = hid_class {
                let capabilities = if class == Class::GameController {
                    vec![
                        cap(
                            "game-controller.input",
                            Kind::Receptor,
                            "按鈕／搖桿／姿態語意事件；不在掃描時讀取",
                            true,
                            false,
                            true,
                        ),
                        cap(
                            "game-controller.haptic",
                            Kind::Actuator,
                            "有硬體上限的震動效果",
                            false,
                            true,
                            true,
                        ),
                        cap(
                            "game-controller.light",
                            Kind::Actuator,
                            "控制器宣告的燈光效果",
                            false,
                            true,
                            true,
                        ),
                    ]
                } else {
                    vec![cap(
                        "hid.semantic-input",
                        Kind::Receptor,
                        "個別授權的語意輸入；不建立全域鍵盤監聽或原始軌跡",
                        true,
                        false,
                        true,
                    )]
                };
                rows.push(DiscoveredHardware {
                    class,
                    display_name: if name.is_empty() {
                        "USB HID".into()
                    } else {
                        name.into()
                    },
                    stable_id: stable_fingerprint("usb-hid", &[serial, vendor, product, name]),
                    identity_basis: "USB serial/vendor/product/name fingerprint（只輸出雜湊）"
                        .into(),
                    availability: Availability::PermissionRequired,
                    permission_requirements: vec!["讀取事件或輸出效果前須逐能力授權".into()],
                    capabilities,
                    source_adapter: "macos-system-profiler".into(),
                    detail:
                        "只列舉 USB registry metadata；未安裝 event tap、未讀取按鍵／游標／姿態"
                            .into(),
                });
            }
        }
    }

    if let Some(bluetooth) = root.get("SPBluetoothDataType").and_then(|v| v.as_array()) {
        for controller in bluetooth {
            for list_key in ["device_connected", "device_not_connected"] {
                let Some(devices) = controller.get(list_key).and_then(|v| v.as_array()) else {
                    continue;
                };
                for wrapper in devices {
                    let Some((name, info)) = wrapper.as_object().and_then(|o| o.iter().next())
                    else {
                        continue;
                    };
                    let services =
                        value_text(info, &["device_services", "device_supportedServices"]);
                    let lower = services.to_lowercase();
                    if !(lower.contains("gatt")
                        || lower.contains("low energy")
                        || lower.contains("ble"))
                    {
                        continue;
                    }
                    let address = value_text(info, &["device_address"]);
                    rows.push(DiscoveredHardware {
                        class: Class::BluetoothLe,
                        display_name: name.clone(),
                        stable_id: stable_fingerprint("ble", &[address, name]),
                        identity_basis: "paired Bluetooth address/name fingerprint（只輸出雜湊）"
                            .into(),
                        availability: Availability::PermissionRequired,
                        permission_requirements: vec![
                            "連線或讀寫 GATT 前需 Bluetooth 與裝置個別授權".into(),
                        ],
                        capabilities: vec![cap(
                            "ble.gatt",
                            Kind::ToolOperation,
                            "已配對 BLE 裝置的受限 GATT 操作",
                            true,
                            true,
                            true,
                        )],
                        source_adapter: "macos-system-profiler".into(),
                        detail: "只讀取已知／已配對裝置 metadata；未開啟 radio discovery 或連線"
                            .into(),
                    });
                }
            }
        }
    }

    for midi in nested_items(root, "SPMIDIDataType") {
        let name = value_text(midi, &["_name"]);
        if name.is_empty() {
            continue;
        }
        let manufacturer = value_text(midi, &["manufacturer", "_manufacturer"]);
        rows.push(DiscoveredHardware {
            class: Class::Midi,
            display_name: name.into(),
            stable_id: stable_fingerprint("midi", &[name, manufacturer]),
            identity_basis: "CoreMIDI declared name/manufacturer fingerprint（只輸出雜湊）".into(),
            availability: Availability::PermissionRequired,
            permission_requirements: vec!["收送 MIDI 訊息前需個別啟用".into()],
            capabilities: vec![
                cap(
                    "midi.input",
                    Kind::Receptor,
                    "MIDI 語意事件",
                    true,
                    false,
                    true,
                ),
                cap(
                    "midi.output",
                    Kind::Actuator,
                    "受限 MIDI 輸出",
                    false,
                    true,
                    true,
                ),
            ],
            source_adapter: "macos-system-profiler".into(),
            detail: "只讀取 CoreMIDI metadata；未開啟 MIDI 連線".into(),
        });
    }

    rows
}

#[cfg(target_os = "linux")]
fn collect_linux_by_id(
    dir: &str,
    class: Class,
    prefix: &str,
    out: &mut Vec<DiscoveredHardware>,
    capabilities: Vec<DiscoveredCapability>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        out.push(DiscoveredHardware {
            class,
            display_name: name.clone(),
            stable_id: Some(format!("linux:{prefix}:{name}")),
            identity_basis: "udev /dev/*/by-id 穩定連結名稱".into(),
            availability: Availability::Available,
            permission_requirements: vec!["啟用或讀寫前仍需個別使用授權".into()],
            capabilities: capabilities.clone(),
            source_adapter: "stable-device-links".into(),
            detail: "只列舉 by-id 中繼資料；未開啟 device node".into(),
        });
    }
}

fn cap(
    id: &str,
    kind: Kind,
    scope: &str,
    read: bool,
    write: bool,
    requires_consent: bool,
) -> DiscoveredCapability {
    DiscoveredCapability {
        id: id.into(),
        kind,
        scope: scope.into(),
        read,
        write,
        requires_consent,
        leaves_device: false,
    }
}

fn all_classes() -> &'static [Class] {
    &[
        Class::Camera,
        Class::Microphone,
        Class::AudioInput,
        Class::AudioOutput,
        Class::Keyboard,
        Class::Mouse,
        Class::Touchpad,
        Class::Tablet,
        Class::GameController,
        Class::Midi,
        Class::UsbSerial,
        Class::BluetoothLe,
        Class::Display,
        Class::SystemNotification,
        Class::OsSensor,
        Class::MdnsDevice,
        Class::Esp32Declaration,
    ]
}

fn unavailable_row(class: Class) -> DiscoveredHardware {
    let (availability, permission, detail) = match class {
        Class::BluetoothLe => (
            Availability::PermissionRequired,
            vec!["必須先由使用者授權 Bluetooth；本次掃描沒有開啟 radio".into()],
            "此 build 沒有 BLE metadata adapter；未掃描附近裝置",
        ),
        Class::MdnsDevice => (
            Availability::PermissionRequired,
            vec!["必須先核准本機網路探索；本次掃描沒有瀏覽 mDNS".into()],
            "未授權本機網路探索時保持關閉",
        ),
        Class::Esp32Declaration => (
            Availability::Unavailable,
            vec!["需由使用者匯入 declarative adapter YAML 並完成配對".into()],
            "可透過宣告式 HTTP/SSE adapter 加入；不由掃描臆測能力",
        ),
        _ => (
            Availability::Unsupported,
            vec![],
            "目前平台 adapter 沒有回報可驗證的穩定身分；不以路徑或假資料冒充裝置",
        ),
    };
    DiscoveredHardware {
        class,
        display_name: format!("{class:?}"),
        stable_id: None,
        identity_basis: "none".into(),
        availability,
        permission_requirements: permission,
        capabilities: vec![],
        source_adapter: "coverage-report".into(),
        detail: detail.into(),
    }
}

pub async fn scan_with_adapters(
    adapters: &[Box<dyn HardwareDiscoveryAdapter>],
) -> HardwareScanReport {
    let started_at = chrono::Utc::now();
    let mut devices = Vec::new();
    let mut limitations = Vec::new();
    for adapter in adapters {
        match adapter.scan_metadata().await {
            Ok(mut rows) => devices.append(&mut rows),
            Err(e) => limitations.push(format!("{}：{e}", adapter.id())),
        }
    }
    let covered: BTreeSet<String> = devices.iter().map(|d| format!("{:?}", d.class)).collect();
    for class in all_classes() {
        if !covered.contains(&format!("{class:?}")) {
            devices.push(unavailable_row(*class));
        }
    }
    HardwareScanReport {
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        started_at,
        completed_at: chrono::Utc::now(),
        sensor_activation_attempted: false,
        devices,
        limitations,
    }
}

impl Runtime {
    pub async fn scan_hardware_capabilities(&self) -> HardwareScanReport {
        let adapters: Vec<Box<dyn HardwareDiscoveryAdapter>> = vec![
            Box::new(SystemPresentationDiscovery),
            Box::new(StableDeviceLinkDiscovery),
            Box::new(MacSystemProfilerDiscovery),
            Box::new(ApprovedLocalDeclarationDiscovery {
                home: self.paths.home.clone(),
            }),
        ];
        let report = scan_with_adapters(&adapters).await;
        let _ = self.store.audit(
            "hardware.metadata-scan",
            "unattributed-local-caller",
            &serde_json::json!({
                "platform": report.platform,
                "rows": report.devices.len(),
                "sensorActivationAttempted": false,
            }),
        );
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneAdapter;
    #[async_trait::async_trait]
    impl HardwareDiscoveryAdapter for OneAdapter {
        fn id(&self) -> &'static str {
            "test"
        }
        async fn scan_metadata(&self) -> Result<Vec<DiscoveredHardware>, String> {
            Ok(vec![DiscoveredHardware {
                class: Class::Camera,
                display_name: "metadata-only camera".into(),
                stable_id: Some("test:camera:stable".into()),
                identity_basis: "test serial".into(),
                availability: Availability::PermissionRequired,
                permission_requirements: vec!["camera".into()],
                capabilities: vec![],
                source_adapter: self.id().into(),
                detail: "not opened".into(),
            }])
        }
    }

    #[tokio::test]
    async fn scan_is_metadata_only_and_reports_every_requested_class() {
        let adapters: Vec<Box<dyn HardwareDiscoveryAdapter>> = vec![Box::new(OneAdapter)];
        let report = scan_with_adapters(&adapters).await;
        assert!(!report.sensor_activation_attempted);
        assert!(report
            .devices
            .iter()
            .any(|d| d.class == Class::Camera && d.stable_id.is_some()));
        for class in all_classes() {
            assert!(
                report.devices.iter().any(|d| d.class == *class),
                "{class:?}"
            );
        }
    }

    #[test]
    fn volatile_paths_never_become_stable_identity() {
        let row = unavailable_row(Class::UsbSerial);
        assert!(row.stable_id.is_none());
        assert_ne!(row.availability, Availability::Available);
    }

    #[test]
    fn macos_profiler_metadata_becomes_stable_capability_rows_without_sensor_use() {
        let fixture = serde_json::json!({
            "SPCameraDataType": [{
                "_name": "Built-in Camera",
                "spcamera_model-id": "camera-model",
                "spcamera_unique-id": "camera-unique-id"
            }],
            "SPAudioDataType": [{"_items": [
                {
                    "_name": "Built-in Microphone",
                    "coreaudio_device_input": 1,
                    "coreaudio_device_manufacturer": "Vendor",
                    "coreaudio_device_transport": "builtin"
                },
                {
                    "_name": "Built-in Speakers",
                    "coreaudio_device_output": 2,
                    "coreaudio_device_manufacturer": "Vendor",
                    "coreaudio_device_transport": "builtin"
                }
            ]}],
            "SPDisplaysDataType": [{
                "_name": "GPU",
                "spdisplays_ndrvs": [{
                    "_name": "Built-in Display",
                    "_spdisplays_display-serial-number": "serial-1",
                    "_spdisplays_display-vendor-id": "vendor-1",
                    "_spdisplays_display-product-id": "product-1"
                }]
            }],
            "SPBluetoothDataType": [{
                "device_connected": [{"Controller": {
                    "device_address": "00:11:22:33:44:55",
                    "device_services": "GATT"
                }}]
            }],
            "SPUSBDataType": [{"_items": [
                {
                    "_name": "Serial Board",
                    "serial_num": "board-serial",
                    "vendor_id": "0x1234",
                    "product_id": "0xabcd"
                },
                {"_name": "USB Hub", "_items": [
                    {"_name": "Studio Keyboard", "vendor_id": "0x1111", "product_id": "0x2222"},
                    {"_name": "Game Controller", "serial_num": "pad-1", "vendor_id": "0x3333", "product_id": "0x4444"}
                ]}
            ]}],
            "SPMIDIDataType": [{"_items": [{
                "_name": "MIDI Controller", "manufacturer": "Vendor"
            }]}]
        });

        let rows = parse_macos_profiler(&fixture);
        for class in [
            Class::Camera,
            Class::Microphone,
            Class::AudioInput,
            Class::AudioOutput,
            Class::Display,
            Class::BluetoothLe,
            Class::UsbSerial,
            Class::Keyboard,
            Class::GameController,
            Class::Midi,
        ] {
            assert!(
                rows.iter().any(|row| {
                    row.class == class
                        && row
                            .stable_id
                            .as_deref()
                            .is_some_and(|id| id.starts_with("macos:"))
                        && !row.capabilities.is_empty()
                }),
                "missing {class:?}"
            );
        }
        assert!(rows.iter().all(|row| {
            !row.stable_id
                .as_deref()
                .unwrap_or_default()
                .contains("00:11:22")
                && !row
                    .stable_id
                    .as_deref()
                    .unwrap_or_default()
                    .contains("camera-unique-id")
        }));
    }

    #[tokio::test]
    async fn approved_local_declaration_is_discovered_without_mdns_browse_or_endpoint_leak() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join("config/adapters");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("esp32.yaml"),
            r#"
schemaVersion: "1.0"
id: esp32-desk
displayName: 桌面精靈裝置
capabilities:
  - kind: receptor
    id: touch
    description: 觸控語意事件
    transport: http
    request: { method: GET, url: "http://desk-spirit.local/touch" }
    facts: { touched: "/touched" }
  - kind: actuator
    id: light
    description: 限亮度狀態燈
    channel: light
    transport: http
    confirmation: acknowledged
    request: { method: POST, url: "http://desk-spirit.local/light" }
"#,
        )
        .unwrap();
        let adapter = ApprovedLocalDeclarationDiscovery {
            home: home.path().to_path_buf(),
        };
        let rows = adapter.scan_metadata().await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.class, Class::Esp32Declaration);
        assert_eq!(row.capabilities.len(), 2);
        assert_eq!(row.availability, Availability::PermissionRequired);
        assert!(row
            .stable_id
            .as_deref()
            .is_some_and(|id| id.starts_with("declaration:esp32:")));
        let visible = serde_json::to_string(row).unwrap();
        assert!(!visible.contains("desk-spirit.local"));
        assert!(row.detail.contains("未瀏覽 mDNS"));
    }
}
