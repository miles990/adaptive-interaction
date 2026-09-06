//! Desktop application use case: a recoverable two-store operation, not ACID.
//! The host serializes windows; the Runtime conditionally writes its own config.
//! No React lifecycle or transport response ordering owns the recovery journal.
use crate::supervisor::{CompletedPresetOp, DesktopPrefs, PendingPresetOp, PendingProactivePatch};
use crate::Backend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewPreset {
    pub preset_id: String,
    pub operation_id: String,
    pub expected_prefs_revision: String,
}

#[derive(Deserialize)]
struct Definition {
    id: String,
    state: PresetValues,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetValues {
    expressiveness: String,
    do_not_disturb: bool,
    proactive_mode: String,
}

fn definition(id: &str) -> Result<Definition, String> {
    serde_json::from_str::<Vec<Definition>>(include_str!(
        "../../src/companion/presetDefinitions.json"
    ))
    .map_err(|_| "preset definitions unavailable".to_string())?
    .into_iter()
    .find(|d| d.id == id)
    .ok_or_else(|| "unknown companion preset".into())
}

pub trait RuntimeSettings: Sync {
    fn read(&self) -> impl std::future::Future<Output = Result<Value, String>> + Send;
    fn configure(
        &self,
        patch: Value,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
}

impl RuntimeSettings for Backend {
    async fn read(&self) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => Ok(rt.proactive_dialogue_status().await),
            Backend::External { base, token } => {
                crate::supervisor::daemon_get(base, token, "/v1/proactive-dialogue").await
            }
        }
    }
    async fn configure(&self, patch: Value) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => rt
                .proactive_dialogue_configure(patch)
                .await
                .map_err(|e| e.to_string()),
            Backend::External { base, token } => {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| e.to_string())?;
                let response = client
                    .patch(format!("{base}/v1/proactive-dialogue"))
                    .bearer_auth(token)
                    .json(&patch)
                    .send()
                    .await
                    .map_err(|_| "runtime unavailable".to_string())?;
                if !response.status().is_success() {
                    return Err(format!(
                        "runtime configuration refused ({})",
                        response.status()
                    ));
                }
                response
                    .json()
                    .await
                    .map_err(|_| "invalid runtime response".into())
            }
        }
    }
}

#[derive(Default)]
pub struct PresetService {
    gate: tokio::sync::Mutex<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetResult {
    pub prefs: DesktopPrefs,
    pub proactive: Option<Value>,
    pub status: &'static str,
    pub error: Option<String>,
    pub cleanup_pending: bool,
}

fn view(
    prefs: &Mutex<DesktopPrefs>,
    proactive: Option<Value>,
    status: &'static str,
    error: Option<&str>,
) -> PresetResult {
    let prefs = prefs.lock().expect("prefs mutex").clone();
    let cleanup_pending = prefs.companion_pending_preset_op.is_some();
    PresetResult {
        prefs,
        proactive,
        status,
        error: error.map(str::to_owned),
        cleanup_pending,
    }
}

fn matches_prefs(prefs: &DesktopPrefs, def: &Definition) -> bool {
    prefs.companion_expressiveness == def.state.expressiveness
        && prefs.companion_do_not_disturb == def.state.do_not_disturb
}

fn mode(snapshot: &Value) -> Option<&str> {
    snapshot.get("config")?.get("mode")?.as_str()
}

impl PresetService {
    /// Busy is explicit: there is no unbounded queue of competing window writes.
    pub fn guard(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        self.gate
            .try_lock()
            .map_err(|_| "陪伴設定正在套用，請稍後再試。".into())
    }

    pub async fn run<R: RuntimeSettings, F: Fn(&DesktopPrefs) -> Result<(), String> + Sync>(
        &self,
        prefs: &Mutex<DesktopPrefs>,
        runtime: &R,
        request: Option<NewPreset>,
        persist: &F,
    ) -> Result<PresetResult, String> {
        let _operation = self.guard()?;
        let initial = match runtime.read().await {
            Ok(value) => value,
            Err(_) => {
                return Ok(view(
                    prefs,
                    None,
                    "unverified",
                    Some("無法讀回主動對話設定，尚未繼續套用。"),
                ))
            }
        };
        if let Some(request) = request {
            let def = definition(&request.preset_id)?;
            if request.operation_id.is_empty() || request.operation_id.len() > 64 {
                return Err("invalid preset operation ID".into());
            }
            let mut current = prefs.lock().expect("prefs mutex");
            if let Some(done) = &current.companion_last_preset_op {
                if done.op_id == request.operation_id {
                    if done.preset_id != request.preset_id {
                        return Err("preset operation ID reused".into());
                    }
                    let status = if matches_prefs(&current, &def)
                        && mode(&initial) == Some(def.state.proactive_mode.as_str())
                    {
                        "applied"
                    } else {
                        "custom-effective"
                    };
                    drop(current);
                    return Ok(view(prefs, Some(initial), status, None));
                }
            }
            if current
                .companion_pending_preset_op
                .as_ref()
                .is_some_and(|op| {
                    op.op_id == request.operation_id && op.preset_id != request.preset_id
                })
            {
                return Err("preset operation ID reused".into());
            }
            let retry = current
                .companion_pending_preset_op
                .as_ref()
                .is_some_and(|op| {
                    op.op_id == request.operation_id && op.preset_id == request.preset_id
                });
            if !retry {
                if request.expected_prefs_revision != current.companion_preset_revision {
                    return Err("桌面設定已變更，請讀回目前設定後再選擇。".into());
                }
                let revision = initial
                    .get("configRevision")
                    .and_then(Value::as_str)
                    .filter(|s| s.parse::<u64>().is_ok())
                    .ok_or("Runtime 版本尚未支援可恢復的陪伴設定；請先更新 Runtime。")?;
                let marker = PendingPresetOp {
                    format: 1,
                    expected_runtime_revision: Some(revision.to_owned()),
                    op_id: request.operation_id,
                    preset_id: request.preset_id,
                    proactive_patch: PendingProactivePatch {
                        mode: def.state.proactive_mode,
                    },
                    issued_at_ms: chrono::Utc::now().timestamp_millis() as f64,
                    ..Default::default()
                };
                crate::commit_prefs_patch(
                    &mut current,
                    &json!({
                        "companionExpressiveness": def.state.expressiveness,
                        "companionDoNotDisturb": def.state.do_not_disturb,
                        "companionPendingPresetOp": marker,
                    }),
                    persist,
                )?;
            }
        }
        let current = prefs.lock().expect("prefs mutex").clone();
        let Some(marker) = current.companion_pending_preset_op.clone() else {
            let status = ["quiet", "natural", "lively"]
                .iter()
                .filter_map(|id| definition(id).ok())
                .any(|d| {
                    matches_prefs(&current, &d)
                        && mode(&initial) == Some(d.state.proactive_mode.as_str())
                });
            return Ok(view(
                prefs,
                Some(initial),
                if status {
                    "applied"
                } else {
                    "custom-effective"
                },
                None,
            ));
        };
        // Invalid/future markers are preserved byte-for-value; they never reset unrelated prefs.
        if marker.malformed.is_some() || marker.format > 1 {
            return Ok(view(
                prefs,
                Some(initial),
                "unverified",
                Some("恢復標記的版本或格式無法辨識，請重新選擇陪伴方式。"),
            ));
        }
        let def = match definition(&marker.preset_id) {
            Ok(def) if def.state.proactive_mode == marker.proactive_patch.mode => def,
            _ => {
                return Ok(view(
                    prefs,
                    Some(initial),
                    "unverified",
                    Some("恢復標記與陪伴方式不一致，請重新選擇。"),
                ))
            }
        };
        if !matches_prefs(&current, &def) {
            self.complete(prefs, &marker, persist)?;
            return Ok(view(prefs, Some(initial), "custom-effective", None));
        }
        if mode(&initial) == Some(marker.proactive_patch.mode.as_str()) {
            let failed = self.complete(prefs, &marker, persist).is_err();
            return Ok(view(
                prefs,
                Some(initial),
                "applied",
                if failed {
                    Some("有效值已確認，恢復標記將在下次啟動時再清理。")
                } else {
                    None
                },
            ));
        }
        let Some(expected) = marker
            .expected_runtime_revision
            .as_deref()
            .filter(|_| marker.format == 1)
        else {
            // v0.7.0 had no Runtime revision. Automatically retrying could overwrite a later human choice.
            return Ok(view(
                prefs,
                Some(initial),
                "unverified",
                Some("舊版恢復標記無法確認設定是否曾變更，請重新選擇陪伴方式。"),
            ));
        };
        if initial.get("configRevision").and_then(Value::as_str) != Some(expected) {
            self.complete(prefs, &marker, persist)?;
            return Ok(view(
                prefs,
                Some(initial),
                "custom-effective",
                Some("主動對話設定已另行修改，保留目前有效值。"),
            ));
        }
        let write = runtime
            .configure(json!({
                "mode": marker.proactive_patch.mode,
                "expectedConfigRevision": expected,
                "operationId": marker.op_id,
            }))
            .await;
        // Always read back, including success. A lost reply isn't a failed write, nor is a reply proof of current effective values.
        let effective = match runtime.read().await {
            Ok(value) => value,
            Err(_) => {
                return Ok(view(
                    prefs,
                    None,
                    "unverified",
                    Some("無法確認生效值，恢復標記已保留。"),
                ))
            }
        };
        if mode(&effective).is_none() {
            return Ok(view(
                prefs,
                Some(effective),
                "unverified",
                Some("讀回資料缺少有效模式，恢復標記已保留。"),
            ));
        }
        if mode(&effective) != Some(marker.proactive_patch.mode.as_str()) {
            return Ok(view(
                prefs,
                Some(effective),
                "partially-applied",
                Some(if write.is_err() {
                    "桌面設定已保存，主動對話尚未完成套用。"
                } else {
                    "主動對話設定已變更，請確認目前有效值。"
                }),
            ));
        }
        let cleanup_failed = self.complete(prefs, &marker, persist).is_err();
        Ok(view(
            prefs,
            Some(effective),
            "applied",
            if cleanup_failed {
                Some("有效值已確認，恢復標記將在下次啟動時再清理。")
            } else {
                None
            },
        ))
    }

    fn complete<F: Fn(&DesktopPrefs) -> Result<(), String>>(
        &self,
        prefs: &Mutex<DesktopPrefs>,
        marker: &PendingPresetOp,
        persist: &F,
    ) -> Result<(), String> {
        let mut current = prefs.lock().expect("prefs mutex");
        if current
            .companion_pending_preset_op
            .as_ref()
            .is_none_or(|op| op.op_id != marker.op_id)
        {
            return Err("preset operation was superseded".into());
        }
        let mut candidate = current.clone();
        candidate.companion_pending_preset_op = None;
        candidate.companion_last_preset_op = Some(CompletedPresetOp {
            op_id: marker.op_id.clone(),
            preset_id: marker.preset_id.clone(),
        });
        persist(&candidate)?;
        *current = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_runtime::{Runtime, RuntimeOptions};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // Fault injection surrounds the production Runtime and atomic prefs store;
    // it never reimplements config writes or the recovery state machine.
    struct Link {
        rt: Runtime,
        reads: AtomicUsize,
        writes: AtomicUsize,
        fail_read_at: AtomicUsize,
        refuse: AtomicBool,
        lose_reply: AtomicBool,
        block: AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    impl Link {
        fn new(rt: Runtime) -> Self {
            Self {
                rt,
                reads: 0.into(),
                writes: 0.into(),
                fail_read_at: usize::MAX.into(),
                refuse: false.into(),
                lose_reply: false.into(),
                block: false.into(),
                entered: Default::default(),
                release: Default::default(),
            }
        }
    }
    impl RuntimeSettings for Link {
        async fn read(&self) -> Result<Value, String> {
            let count = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
            if count == self.fail_read_at.load(Ordering::SeqCst) {
                return Err("read disconnected".into());
            }
            Ok(self.rt.proactive_dialogue_status().await)
        }
        async fn configure(&self, patch: Value) -> Result<Value, String> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.block.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.refuse.load(Ordering::SeqCst) {
                return Err("runtime temporarily busy".into());
            }
            let result = self
                .rt
                .proactive_dialogue_configure(patch)
                .await
                .map_err(|e| e.to_string())?;
            if self.lose_reply.load(Ordering::SeqCst) {
                Err("reply lost after durable commit".into())
            } else {
                Ok(result)
            }
        }
    }
    async fn start(home: &std::path::Path) -> Runtime {
        Runtime::start(RuntimeOptions {
            home: Some(home.to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap()
    }
    fn request(prefs: &Mutex<DesktopPrefs>, id: &str) -> Option<NewPreset> {
        Some(NewPreset {
            preset_id: "quiet".into(),
            operation_id: id.into(),
            expected_prefs_revision: prefs.lock().unwrap().companion_preset_revision.clone(),
        })
    }
    fn reload(path: &std::path::Path) -> Mutex<DesktopPrefs> {
        Mutex::new(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
    }

    #[tokio::test]
    async fn first_commit_failure_has_no_runtime_or_memory_effect() {
        let dir = tempfile::tempdir().unwrap();
        let link = Link::new(start(dir.path()).await);
        let prefs = Mutex::new(DesktopPrefs::default());
        let before = serde_json::to_value(prefs.lock().unwrap().clone()).unwrap();
        let result = PresetService::default()
            .run(&prefs, &link, request(&prefs, "a"), &|_| {
                Err("read only".into())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(link.writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            serde_json::to_value(prefs.lock().unwrap().clone()).unwrap(),
            before
        );
        link.rt.shutdown().await;
    }

    #[tokio::test]
    async fn partial_stage_is_durable_and_new_host_recovers_only_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.json");
        let link = Link::new(start(dir.path()).await);
        link.rt
            .proactive_dialogue_configure(
                json!({"mode":"off", "dailyGenerativeCostUsd":0.25, "maxPerHour":1}),
            )
            .await
            .unwrap();
        let before = link.rt.proactive_dialogue_status().await;
        let prefs = Mutex::new(DesktopPrefs::default());
        let save = |p: &DesktopPrefs| crate::supervisor::save_prefs_at(&path, p);
        link.refuse.store(true, Ordering::SeqCst);
        let partial = PresetService::default()
            .run(&prefs, &link, request(&prefs, "a"), &save)
            .await
            .unwrap();
        assert_eq!(partial.status, "partially-applied");
        assert!(partial.cleanup_pending);
        let persisted = reload(&path);
        assert_eq!(persisted.lock().unwrap().companion_expressiveness, "quiet");
        assert_eq!(
            persisted
                .lock()
                .unwrap()
                .companion_pending_preset_op
                .as_ref()
                .unwrap()
                .op_id,
            "a"
        );
        link.rt.shutdown().await;
        let restarted = Link::new(start(dir.path()).await);
        let result = PresetService::default()
            .run(&persisted, &restarted, None, &save)
            .await
            .unwrap();
        assert_eq!(result.status, "applied");
        assert!(!result.cleanup_pending);
        assert_eq!(restarted.writes.load(Ordering::SeqCst), 1);
        let after = restarted.rt.proactive_dialogue_status().await;
        for key in before["config"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.as_str() != "mode")
        {
            assert_eq!(
                before["config"][key], after["config"][key],
                "preset must preserve {key}"
            );
        }
        assert!(reload(&path)
            .lock()
            .unwrap()
            .companion_pending_preset_op
            .is_none());
        restarted.rt.shutdown().await;
    }

    #[tokio::test]
    async fn lost_reply_readback_confirms_once_and_duplicate_operation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let link = Link::new(start(dir.path()).await);
        let prefs = Mutex::new(DesktopPrefs::default());
        link.lose_reply.store(true, Ordering::SeqCst);
        let service = PresetService::default();
        let result = service
            .run(&prefs, &link, request(&prefs, "a"), &|_| Ok(()))
            .await
            .unwrap();
        assert_eq!(result.status, "applied");
        let revision = link.rt.proactive_dialogue_status().await["configRevision"].clone();
        let duplicate = service
            .run(&prefs, &link, request(&prefs, "a"), &|_| {
                panic!("duplicate must not write prefs")
            })
            .await
            .unwrap();
        assert_eq!(duplicate.status, "applied");
        assert_eq!(link.writes.load(Ordering::SeqCst), 1);
        assert_eq!(
            link.rt.proactive_dialogue_status().await["configRevision"],
            revision
        );
        link.rt.shutdown().await;
    }

    #[tokio::test]
    async fn successful_write_failed_readback_recovers_after_both_stores_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.json");
        let link = Link::new(start(dir.path()).await);
        let prefs = Mutex::new(DesktopPrefs::default());
        let save = |p: &DesktopPrefs| crate::supervisor::save_prefs_at(&path, p);
        link.fail_read_at.store(2, Ordering::SeqCst);
        let result = PresetService::default()
            .run(&prefs, &link, request(&prefs, "a"), &save)
            .await
            .unwrap();
        assert_eq!(result.status, "unverified");
        assert!(result.cleanup_pending);
        link.rt.shutdown().await;
        let restarted = Link::new(start(dir.path()).await);
        let result = PresetService::default()
            .run(&reload(&path), &restarted, None, &save)
            .await
            .unwrap();
        assert_eq!(result.status, "applied");
        assert_eq!(restarted.writes.load(Ordering::SeqCst), 0);
        restarted.rt.shutdown().await;
    }

    #[tokio::test]
    async fn cleanup_failure_retains_marker_but_verified_values_are_applied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.json");
        let link = Link::new(start(dir.path()).await);
        let prefs = Mutex::new(DesktopPrefs::default());
        let count = AtomicUsize::new(0);
        let result = PresetService::default()
            .run(&prefs, &link, request(&prefs, "a"), &|p| {
                if count.fetch_add(1, Ordering::SeqCst) == 1 {
                    Err("cleanup write failed".into())
                } else {
                    crate::supervisor::save_prefs_at(&path, p)
                }
            })
            .await
            .unwrap();
        assert_eq!(result.status, "applied");
        assert!(result.cleanup_pending && result.error.is_some());
        let recovered = PresetService::default()
            .run(&reload(&path), &link, None, &|p| {
                crate::supervisor::save_prefs_at(&path, p)
            })
            .await
            .unwrap();
        assert!(!recovered.cleanup_pending);
        assert_eq!(link.writes.load(Ordering::SeqCst), 1);
        link.rt.shutdown().await;
    }

    #[tokio::test]
    async fn newer_runtime_choice_or_unrelated_runtime_config_is_never_overwritten() {
        for patch in [json!({"mode":"off"}), json!({"maxPerHour":1})] {
            let dir = tempfile::tempdir().unwrap();
            let link = Link::new(start(dir.path()).await);
            let prefs = Mutex::new(DesktopPrefs::default());
            link.refuse.store(true, Ordering::SeqCst);
            let service = PresetService::default();
            service
                .run(&prefs, &link, request(&prefs, "a"), &|_| Ok(()))
                .await
                .unwrap();
            link.rt.proactive_dialogue_configure(patch).await.unwrap();
            let before = link.rt.proactive_dialogue_status().await;
            link.refuse.store(false, Ordering::SeqCst);
            let result = service.run(&prefs, &link, None, &|_| Ok(())).await.unwrap();
            assert_eq!(result.status, "custom-effective");
            assert_eq!(link.rt.proactive_dialogue_status().await, before);
            assert_eq!(link.writes.load(Ordering::SeqCst), 1);
            link.rt.shutdown().await;
        }
    }

    #[tokio::test]
    async fn prefs_patch_cancels_related_intent_but_preserves_unrelated_choice() {
        for related in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let link = Link::new(start(dir.path()).await);
            let prefs = Mutex::new(DesktopPrefs::default());
            link.refuse.store(true, Ordering::SeqCst);
            let service = PresetService::default();
            service
                .run(&prefs, &link, request(&prefs, "a"), &|_| Ok(()))
                .await
                .unwrap();
            crate::commit_prefs_patch(
                &mut prefs.lock().unwrap(),
                &if related {
                    json!({"companionExpressiveness":"lively"})
                } else {
                    json!({"companionOpacity":0.8})
                },
                |_| Ok(()),
            )
            .unwrap();
            link.refuse.store(false, Ordering::SeqCst);
            let result = service.run(&prefs, &link, None, &|_| Ok(())).await.unwrap();
            if related {
                assert_eq!(result.status, "custom-effective");
                assert_eq!(result.prefs.companion_expressiveness, "lively");
                assert_eq!(link.writes.load(Ordering::SeqCst), 1);
            } else {
                assert_eq!(result.status, "applied");
                assert_eq!(result.prefs.companion_opacity, 0.8);
            }
            link.rt.shutdown().await;
        }
    }

    #[tokio::test]
    async fn concurrent_window_is_busy_then_stale_view_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let link = Link::new(start(dir.path()).await);
        let prefs = Mutex::new(DesktopPrefs::default());
        let service = PresetService::default();
        let stale = request(&prefs, "b");
        link.block.store(true, Ordering::SeqCst);
        let first = service.run(&prefs, &link, request(&prefs, "a"), &|_| Ok(()));
        let second = async {
            link.entered.notified().await;
            assert!(service
                .run(&prefs, &link, request(&prefs, "b"), &|_| Ok(()))
                .await
                .unwrap_err()
                .contains("稍後"));
            assert!(service.guard().is_err());
            link.release.notify_one();
        };
        let (result, ()) = tokio::join!(first, second);
        assert_eq!(result.unwrap().status, "applied");
        assert!(service
            .run(&prefs, &link, stale, &|_| Ok(()))
            .await
            .unwrap_err()
            .contains("已變更"));
        link.rt.shutdown().await;
    }

    #[tokio::test]
    async fn runtime_revision_is_compare_and_swap_and_receipt_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let rt = start(dir.path()).await;
        let patch = json!({"mode":"necessary", "expectedConfigRevision":"0", "operationId":"one"});
        let result = rt
            .proactive_dialogue_configure(patch.clone())
            .await
            .unwrap();
        assert_eq!(result["configRevision"], "1");
        assert_eq!(
            rt.proactive_dialogue_configure(patch.clone())
                .await
                .unwrap(),
            result
        );
        assert!(rt
            .proactive_dialogue_configure(
                json!({"mode":"off", "expectedConfigRevision":"0", "operationId":"two"})
            )
            .await
            .is_err());
        assert!(rt
            .proactive_dialogue_configure(
                json!({"mode":"off", "expectedConfigRevision":"1", "operationId":"one"})
            )
            .await
            .is_err());
        rt.shutdown().await;
        let rt = start(dir.path()).await;
        assert_eq!(
            rt.proactive_dialogue_configure(patch.clone())
                .await
                .unwrap()["configRevision"],
            "1"
        );
        rt.proactive_dialogue_configure(json!({"mode":"off"}))
            .await
            .unwrap();
        assert!(rt.proactive_dialogue_configure(patch).await.is_err());
        assert_eq!(
            rt.proactive_dialogue_status().await["config"]["mode"],
            "off"
        );
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn malformed_future_and_legacy_markers_preserve_prefs_without_unsafe_retry() {
        for raw in [
            json!("corrupt"),
            json!({"format":99,"future":{"intent":2}}),
            json!({"opId":"old","presetId":"quiet","proactivePatch":{"mode":"necessary"},"issuedAtMs":1.0}),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let link = Link::new(start(dir.path()).await);
            let mut value = serde_json::to_value(DesktopPrefs::default()).unwrap();
            value["companionExpressiveness"] = json!("quiet");
            value["companionDoNotDisturb"] = json!(true);
            value["companionPendingPresetOp"] = raw.clone();
            value["companionOpacity"] = json!(0.75);
            let prefs = Mutex::new(serde_json::from_value(value).unwrap());
            let result = PresetService::default()
                .run(&prefs, &link, None, &|_| {
                    panic!("unknown marker must be preserved")
                })
                .await
                .unwrap();
            assert_eq!(result.status, "unverified");
            assert_eq!(
                serde_json::to_value(&result.prefs).unwrap()["companionPendingPresetOp"],
                raw
            );
            assert_eq!(result.prefs.companion_opacity, 0.75);
            assert_eq!(link.writes.load(Ordering::SeqCst), 0);
            crate::commit_prefs_patch(
                &mut prefs.lock().unwrap(),
                &json!({"companionOpacity":0.9}),
                |_| Ok(()),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(prefs.lock().unwrap().clone()).unwrap()
                    ["companionPendingPresetOp"],
                raw
            );
            let selected = PresetService::default()
                .run(
                    &prefs,
                    &link,
                    request(&prefs, "new-explicit-choice"),
                    &|_| Ok(()),
                )
                .await
                .unwrap();
            assert_eq!(selected.status, "applied");
            link.rt.shutdown().await;
        }
    }

    #[tokio::test]
    async fn unreadable_runtime_makes_no_store_changes() {
        let dir = tempfile::tempdir().unwrap();
        let link = Link::new(start(dir.path()).await);
        link.fail_read_at.store(1, Ordering::SeqCst);
        let prefs = Mutex::new(DesktopPrefs::default());
        let result = PresetService::default()
            .run(&prefs, &link, request(&prefs, "a"), &|_| {
                panic!("must not commit without revision")
            })
            .await
            .unwrap();
        assert_eq!(result.status, "unverified");
        assert!(!result.cleanup_pending);
        assert_eq!(link.writes.load(Ordering::SeqCst), 0);
        link.rt.shutdown().await;
    }
}
