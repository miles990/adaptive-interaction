//! High-sensitivity media receptors. Privacy properties enforced in code:
//!
//! - DEFAULT OFF + consent-gated (`requires_consent = true`, Intimate).
//! - Raw audio NEVER leaves process memory: only derived facts (RMS level,
//!   speaking yes/no, duration) become observations. No samples are stored,
//!   no STT runs, nothing is sent anywhere.
//! - Every listen window has a HARD auto-stop deadline.
//! - Stop/emergency-stop releases the device immediately.
//! - The active/inactive state is reported through a callback so the runtime
//!   can drive always-visible indicators (tray / companion / control center).
//!
//! Camera presence detection is NOT implemented in this build: the honest
//! state is "no camera driver", never a mock pretending to see you.

use async_trait::async_trait;
use chrono::Utc;
use interaction_adapter_sdk::ReceptorManifestBuilder;
use interaction_core::{
    ComponentHealth, DataRetention, DataSemantics, DataSensitivityLevel, DataSource, HumanMeta,
    Observation, Receptor, ReceptorError, ReceptorId, ReceptorManifest, ReceptorMode, Sensitivity,
    SessionContext, TriState,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Hard ceiling for one listen window.
pub const MAX_LISTEN_MS: u64 = 30_000;

/// Callback the runtime installs to mirror capture state into indicators.
pub type SensorStateCallback = Arc<dyn Fn(&str, bool) + Send + Sync>;

/// Abstract capture source: real cpal, or a deterministic fake for tests.
pub trait CaptureSource: Send + Sync {
    /// Begin capturing; returns a handle that yields RMS levels (0..1) and
    /// stops capturing when dropped.
    fn start(&self) -> Result<Box<dyn CaptureHandle>, String>;
    fn available(&self) -> bool;
}

pub trait CaptureHandle: Send {
    /// Latest RMS level 0..1 (None until the first buffer arrives).
    fn level(&self) -> Option<f64>;
}

// ---------------------------------------------------------------------------
// Real cpal source (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "mic-capture")]
pub mod cpal_source {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    pub struct CpalSource;

    struct CpalHandle {
        // Held only for its Drop: dropping the stream releases the device.
        _stream: cpal::Stream,
    }

    // SAFETY-adjacent honesty note: cpal::Stream is !Send on some platforms;
    // we therefore keep the stream on a dedicated thread and only share the
    // level cell.
    pub struct ThreadedHandle {
        level: Arc<Mutex<Option<f64>>>,
        stop: Arc<AtomicBool>,
    }

    impl CaptureHandle for ThreadedHandle {
        fn level(&self) -> Option<f64> {
            *self.level.lock().expect("level lock")
        }
    }

    impl Drop for ThreadedHandle {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    impl CaptureSource for CpalSource {
        fn available(&self) -> bool {
            cpal::default_host().default_input_device().is_some()
        }

        fn start(&self) -> Result<Box<dyn CaptureHandle>, String> {
            let level = Arc::new(Mutex::new(None));
            let stop = Arc::new(AtomicBool::new(false));
            let level_thread = level.clone();
            let stop_thread = stop.clone();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
            std::thread::spawn(move || {
                let run = || -> Result<cpal::Stream, String> {
                    let device = cpal::default_host()
                        .default_input_device()
                        .ok_or("no input device")?;
                    let config = device.default_input_config().map_err(|e| e.to_string())?;
                    let level_cb = level_thread.clone();
                    let stream = device
                        .build_input_stream(
                            &config.into(),
                            move |data: &[f32], _| {
                                // Derived level only; samples are dropped here.
                                let rms = (data.iter().map(|s| s * s).sum::<f32>()
                                    / data.len().max(1) as f32)
                                    .sqrt();
                                *level_cb.lock().expect("level lock") =
                                    Some((rms as f64).clamp(0.0, 1.0));
                            },
                            |e| tracing::warn!(error = %e, "mic stream error"),
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    stream.play().map_err(|e| e.to_string())?;
                    Ok(stream)
                };
                match run() {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(()));
                        let handle = CpalHandle { _stream: stream };
                        while !stop_thread.load(Ordering::SeqCst) {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        drop(handle); // releases the device
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            });
            match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(Ok(())) => Ok(Box::new(ThreadedHandle { level, stop })),
                Ok(Err(e)) => Err(format!("microphone unavailable: {e}")),
                Err(_) => Err("microphone start timed out".into()),
            }
        }
    }
}

/// Honest "no backend" source: builds without the capture feature report
/// themselves unavailable instead of pretending.
pub struct UnavailableSource;

impl CaptureSource for UnavailableSource {
    fn available(&self) -> bool {
        false
    }

    fn start(&self) -> Result<Box<dyn CaptureHandle>, String> {
        Err("no capture backend compiled into this build".into())
    }
}

/// Deterministic fake source for tests (no device, no permissions).
pub struct FakeSource {
    pub available: bool,
    pub level: f64,
    pub started: Arc<AtomicBool>,
    pub stopped: Arc<AtomicBool>,
}

impl FakeSource {
    pub fn new(level: f64) -> Self {
        Self {
            available: true,
            level,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct FakeHandle {
    level: f64,
    stopped: Arc<AtomicBool>,
}

impl CaptureHandle for FakeHandle {
    fn level(&self) -> Option<f64> {
        Some(self.level)
    }
}

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

impl CaptureSource for FakeSource {
    fn available(&self) -> bool {
        self.available
    }

    fn start(&self) -> Result<Box<dyn CaptureHandle>, String> {
        self.started.store(true, Ordering::SeqCst);
        Ok(Box::new(FakeHandle {
            level: self.level,
            stopped: self.stopped.clone(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Microphone listen receptor (click-to-listen window)
// ---------------------------------------------------------------------------

pub struct MicListenReceptor {
    source: Mutex<Arc<dyn CaptureSource>>,
    active: Mutex<Option<ActiveListen>>,
    on_state: Option<SensorStateCallback>,
    max_listen_ms: u64,
}

struct ActiveListen {
    handle: Box<dyn CaptureHandle>,
    started_at: chrono::DateTime<Utc>,
    deadline: chrono::DateTime<Utc>,
}

impl MicListenReceptor {
    pub fn new(source: Arc<dyn CaptureSource>, on_state: Option<SensorStateCallback>) -> Self {
        Self {
            source: Mutex::new(source),
            active: Mutex::new(None),
            on_state,
            max_listen_ms: MAX_LISTEN_MS,
        }
    }

    /// Test/diagnostic injection: swap the capture backend (never mid-listen).
    pub fn swap_source(&self, source: Arc<dyn CaptureSource>) {
        self.stop_listen();
        *self.source.lock().expect("source lock") = source;
    }

    fn current_source(&self) -> Arc<dyn CaptureSource> {
        self.source.lock().expect("source lock").clone()
    }

    fn notify(&self, active: bool) {
        if let Some(cb) = &self.on_state {
            cb("microphone", active);
        }
    }

    /// Begin one bounded listen window (click-to-listen). Refuses when a
    /// window is already open.
    pub fn begin_listen(&self, duration_ms: u64) -> Result<(), ReceptorError> {
        let mut active = self.active.lock().expect("active lock");
        if active.is_some() {
            return Err(ReceptorError::Unavailable(
                "already listening; stop first".into(),
            ));
        }
        let handle = self
            .current_source()
            .start()
            .map_err(ReceptorError::Unavailable)?;
        let now = Utc::now();
        let bounded = duration_ms.clamp(500, self.max_listen_ms);
        *active = Some(ActiveListen {
            handle,
            started_at: now,
            deadline: now + chrono::Duration::milliseconds(bounded as i64),
        });
        drop(active);
        self.notify(true);
        Ok(())
    }

    /// Stop capturing NOW (user action / estop / deadline).
    pub fn stop_listen(&self) {
        let had = self.active.lock().expect("active lock").take();
        if had.is_some() {
            self.notify(false);
        }
    }

    pub fn is_listening(&self) -> bool {
        self.enforce_deadline();
        self.active.lock().expect("active lock").is_some()
    }

    fn enforce_deadline(&self) {
        let expired = {
            let active = self.active.lock().expect("active lock");
            active
                .as_ref()
                .map(|a| Utc::now() >= a.deadline)
                .unwrap_or(false)
        };
        if expired {
            self.stop_listen();
        }
    }
}

#[async_trait]
impl Receptor for MicListenReceptor {
    fn manifest(&self) -> ReceptorManifest {
        let available = self.current_source().available();
        let mut b = ReceptorManifestBuilder::new(
            "microphone.listen",
            "Microphone (click-to-listen)",
            "media.microphone",
        )
        .description(
            "Bounded listen windows producing sound-level facts only. Raw audio stays in \
             memory, is never stored and never leaves this machine; no speech-to-text.",
        )
        .category("sensor")
        .provides(&["listening", "level", "speaking", "windowMs"])
        .mode(ReceptorMode::Poll)
        .sensitivity(Sensitivity::Intimate, true)
        .human(HumanMeta {
            data: Some(DataSemantics {
                data_categories: vec!["sound-level".into()],
                personal_data: TriState::Yes,
                sensitivity: DataSensitivityLevel::High,
                source: DataSource::Local,
                leaves_device: TriState::No,
                retention: DataRetention::None,
                fact_fields: vec!["level".into(), "speaking".into()],
                inference_fields: vec![],
            }),
            ..Default::default()
        });
        if !available {
            b = b.description(
                "Microphone capture is not available in this build/host. \
                 The receptor reports itself unavailable instead of pretending.",
            );
        }
        let mut m = b.build();
        if !available {
            m.availability = interaction_core::Availability::Offline;
        }
        m
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        // Enabling the receptor does NOT open the microphone; only an
        // explicit begin_listen does.
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        self.enforce_deadline();
        let active = self.active.lock().expect("active lock");
        let mut obs = Observation::now(
            ReceptorId::new("microphone.listen"),
            "media.microphone",
            Utc::now(),
        );
        match active.as_ref() {
            Some(listen) => {
                let level = listen.handle.level().unwrap_or(0.0);
                obs.facts.insert("listening".into(), json!(true));
                obs.facts.insert("level".into(), json!(level));
                obs.facts.insert("speaking".into(), json!(level > 0.06));
                obs.facts.insert(
                    "windowMs".into(),
                    json!(Utc::now()
                        .signed_duration_since(listen.started_at)
                        .num_milliseconds()),
                );
            }
            None => {
                obs.facts.insert("listening".into(), json!(false));
            }
        }
        Ok(obs)
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        self.stop_listen();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receptor_with(level: f64) -> (MicListenReceptor, Arc<FakeSource>) {
        let source = Arc::new(FakeSource::new(level));
        let states: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(vec![]));
        let states_cb = states.clone();
        let cb: SensorStateCallback = Arc::new(move |kind, on| {
            states_cb.lock().unwrap().push((kind.to_string(), on));
        });
        (MicListenReceptor::new(source.clone(), Some(cb)), source)
    }

    #[tokio::test]
    async fn enabling_never_opens_the_microphone() {
        let (r, source) = receptor_with(0.2);
        r.start(SessionContext {
            session_id: interaction_core::SessionId::new("s"),
        })
        .await
        .unwrap();
        assert!(
            !source.started.load(Ordering::SeqCst),
            "start() must not capture"
        );
        let obs = r.read().await.unwrap();
        assert_eq!(obs.facts["listening"], json!(false));
        assert!(
            !source.started.load(Ordering::SeqCst),
            "read() must not capture"
        );
    }

    #[tokio::test]
    async fn listen_window_produces_level_facts_only_and_stops_on_deadline() {
        let (r, source) = receptor_with(0.3);
        r.begin_listen(600).unwrap();
        assert!(source.started.load(Ordering::SeqCst));
        let obs = r.read().await.unwrap();
        assert_eq!(obs.facts["listening"], json!(true));
        assert_eq!(obs.facts["speaking"], json!(true));
        // Facts are DERIVED only — no raw samples anywhere in the observation.
        assert!(!obs.facts.contains_key("samples"));
        assert!(!obs.facts.contains_key("audio"));
        // Deadline: clamped to at least 500ms; wait it out.
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        assert!(!r.is_listening(), "hard deadline must auto-stop capture");
        assert!(source.stopped.load(Ordering::SeqCst), "device released");
    }

    #[tokio::test]
    async fn stop_releases_device_immediately() {
        let (r, source) = receptor_with(0.1);
        r.begin_listen(10_000).unwrap();
        r.stop().await.unwrap();
        assert!(source.stopped.load(Ordering::SeqCst));
        assert!(!r.is_listening());
    }

    #[test]
    fn manifest_is_consent_gated_high_sensitivity_local_only() {
        let (r, _) = receptor_with(0.0);
        let m = r.manifest();
        assert!(m.requires_consent);
        assert_eq!(m.sensitivity, Sensitivity::Intimate);
        let data = m.human.as_ref().unwrap().data.as_ref().unwrap();
        assert_eq!(data.leaves_device, TriState::No);
        assert_eq!(data.retention, DataRetention::None);
    }

    #[test]
    fn double_listen_is_refused() {
        let (r, _) = receptor_with(0.0);
        r.begin_listen(5_000).unwrap();
        assert!(r.begin_listen(5_000).is_err());
    }
}
