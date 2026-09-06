//! Independent N4 review regressions through the production preference writer.

#[cfg(unix)]
#[test]
fn atomic_preferences_save_preserves_existing_private_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("desktop.json");
    std::fs::write(&path, b"{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let prefs = crate::supervisor::DesktopPrefs {
        companion_name: "private display name".into(),
        ..Default::default()
    };
    crate::supervisor::save_prefs_at(&path, &prefs).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600,
        "atomic replacement must not widen the access mode of an existing private preferences file"
    );
}

#[test]
fn failed_temporary_creation_does_not_delete_a_file_the_writer_did_not_create() {
    const CHILD: &str = "AIP_SETTINGS_REVIEW_COLLISION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // A fresh test process gives the production per-process sequence its
        // deterministic initial value, without changing shared process state.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings_recovery_review::failed_temporary_creation_does_not_delete_a_file_the_writer_did_not_create",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("desktop.json");
    let temporary = path.with_extension(format!("json.tmp.{}.0", std::process::id()));
    std::fs::write(&path, b"original preferences").unwrap();
    std::fs::write(&temporary, b"pre-existing unrelated file").unwrap();
    let result = crate::supervisor::save_prefs_at(&path, &Default::default());
    assert!(result.is_err(), "create_new must reject the collision");
    assert_eq!(std::fs::read(&path).unwrap(), b"original preferences");
    assert_eq!(
        std::fs::read(&temporary).ok().as_deref(),
        Some(b"pre-existing unrelated file".as_slice()),
        "a failed create_new did not grant ownership of the existing temporary path"
    );
}
