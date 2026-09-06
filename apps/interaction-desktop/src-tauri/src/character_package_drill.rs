//! N5 process-local production-port drill. No native window or device acceptance.
use crate::{character_bridge, character_store, supervisor};
use interaction_character::{CharacterManifest, Negotiate};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::path::Path;

const PACKAGE: &str = include_str!("../../../../examples/characters/drill-text/manifest.json");
const FALLBACK: &str = include_str!("../../public/characters/plain-text/manifest.json");

async fn start(home: &Path) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .expect("isolated runtime starts")
}

async fn hello(rt: &Runtime, text: &str) {
    let manifest: CharacterManifest = serde_json::from_str(text).unwrap();
    character_bridge::hello(
        rt,
        json!({
            "manifest": manifest,
            "negotiate": Negotiate::from_manifest(&manifest, 1),
            "visible": true,
            "packId": manifest.character_id,
            "reducedMotion": true
        }),
    )
    .await
    .expect("real runtime Character hello accepts the package");
}

fn prefs_at(path: &Path) -> supervisor::DesktopPrefs {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn package_import_remove_fallback_preserves_preferences_audit_and_session_after_restart() {
    let home = tempfile::tempdir().unwrap();
    let root = character_store::characters_root(home.path());
    let prefs_path = home.path().join("state/desktop.json");
    let imported = character_store::import(&root, PACKAGE, &[]).unwrap();
    assert_eq!(imported.character_id, "drill-text");
    let list = character_store::list(&root);
    assert_eq!(list.len(), 1);
    assert!(list[0].valid);
    assert_eq!(list[0].entrypoint.as_deref(), Some("text"));
    assert!(!list[0].executable && !list[0].network && !list[0].external);
    let mut prefs = supervisor::DesktopPrefs {
        companion_pack: "drill-text".into(),
        companion_sound: false,
        companion_do_not_disturb: true,
        companion_preferences: serde_json::from_value(json!({
            "drill-text": {"motto": "保留我的偏好"},
            "plain-text": {"motto": "另一個角色"}
        }))
        .unwrap(),
        ..Default::default()
    };
    supervisor::save_prefs_at(&prefs_path, &prefs).unwrap();
    let rt = start(home.path()).await;
    hello(&rt, PACKAGE).await;
    assert_eq!(
        character_bridge::manifest(&rt).await.unwrap()["characterId"],
        "drill-text"
    );
    let before = rt.character_session_peek().unwrap();
    let audit_before = rt.store.audit_tail(100).unwrap();
    assert!(audit_before
        .iter()
        .any(|row| row["kind"] == "character.hello"));

    // This is the same production entry used by character_remove IPC. The store
    // owns files only; the UI separately chooses/persists the fallback below.
    character_store::remove(&root, "drill-text").unwrap();
    assert!(character_store::list(&root).is_empty());
    assert!(!root.join("drill-text").exists());
    assert!(character_store::remove(&root, "plain-text").is_err());
    assert_eq!(rt.character_session_peek().unwrap().hash, before.hash);
    assert_eq!(rt.store.audit_tail(100).unwrap(), audit_before);
    assert_eq!(
        prefs_at(&prefs_path).companion_pack,
        "drill-text",
        "removal alone never fabricates a fallback acknowledgment"
    );

    // Explicit host fallback application, mirroring the separately exercised
    // CompanionPage remove→select flow. Package preferences remain recoverable.
    prefs.companion_pack = "plain-text".into();
    supervisor::save_prefs_at(&prefs_path, &prefs).unwrap();
    hello(&rt, FALLBACK).await;
    assert_eq!(
        character_bridge::manifest(&rt).await.unwrap()["characterId"],
        "plain-text"
    );
    let after = rt.character_session_peek().unwrap();
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.state["truth"], before.state["truth"]);
    assert!(after.revision >= before.revision);
    let durable_audits = rt.store.audit_tail(100).unwrap();
    rt.shutdown().await;
    drop(rt);

    let restored_prefs = prefs_at(&prefs_path);
    assert_eq!(restored_prefs.companion_pack, "plain-text");
    assert_eq!(
        restored_prefs.companion_preferences,
        prefs.companion_preferences
    );
    assert!(restored_prefs.companion_do_not_disturb);
    assert!(!restored_prefs.companion_sound);
    let rt = start(home.path()).await;
    let restored = rt.character_session_peek().unwrap();
    assert_eq!(restored.session_id, after.session_id);
    assert_eq!(restored.epoch, after.epoch);
    assert!(restored.revision >= after.revision);
    assert_eq!(restored.state["truth"], after.state["truth"]);
    let audits: Vec<Value> = rt.store.audit_tail(200).unwrap();
    for row in durable_audits {
        assert!(
            audits.contains(&row),
            "historical audit remains after restart"
        );
    }
    assert!(character_store::list(&root).is_empty());
    hello(&rt, FALLBACK).await;
    assert_eq!(
        character_bridge::manifest(&rt).await.unwrap()["characterId"],
        "plain-text"
    );
    rt.shutdown().await;
}
