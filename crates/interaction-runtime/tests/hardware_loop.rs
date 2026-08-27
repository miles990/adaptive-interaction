//! Metadata-only hardware scan contract: no sensor activation, complete class
//! coverage, stable identity honesty, and no mutation of runtime sensor truth.

use interaction_core::{HardwareAvailability, HardwareClass};
use interaction_runtime::{Runtime, RuntimeOptions};

#[tokio::test]
async fn hardware_scan_never_activates_sensors_and_reports_every_class() {
    let home = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();

    assert!(rt.active_sensors().is_empty());
    let report = rt.scan_hardware_capabilities().await;
    assert!(!report.sensor_activation_attempted);
    assert!(
        rt.active_sensors().is_empty(),
        "掃描不得改變 Runtime sensor truth"
    );
    assert!(!report.devices.is_empty());

    for class in [
        HardwareClass::Camera,
        HardwareClass::Microphone,
        HardwareClass::AudioInput,
        HardwareClass::AudioOutput,
        HardwareClass::Keyboard,
        HardwareClass::Mouse,
        HardwareClass::Touchpad,
        HardwareClass::Tablet,
        HardwareClass::GameController,
        HardwareClass::Midi,
        HardwareClass::UsbSerial,
        HardwareClass::BluetoothLe,
        HardwareClass::Display,
        HardwareClass::SystemNotification,
        HardwareClass::OsSensor,
        HardwareClass::MdnsDevice,
        HardwareClass::Esp32Declaration,
    ] {
        assert!(report.devices.iter().any(|d| d.class == class), "{class:?}");
    }

    for device in &report.devices {
        if device.stable_id.is_none() {
            assert_ne!(device.availability, HardwareAvailability::Available);
        }
    }

    let audit = rt.store.audit_tail(10).unwrap();
    assert!(audit.iter().any(|row| {
        row.get("kind").and_then(|v| v.as_str()) == Some("hardware.metadata-scan")
            && row
                .get("detail")
                .and_then(|v| v.get("sensorActivationAttempted"))
                .and_then(|v| v.as_bool())
                == Some(false)
    }));
}
