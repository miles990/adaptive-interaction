use interaction_session::{CharacterSession,SessionConfig,state_hash};
use serde_json::Value;
#[test]
fn complete_snapshot_positive_control() {
 let cfg=SessionConfig::default();let now="2026-09-06T00:00:00Z".parse().unwrap();
 let snap=CharacterSession::new(cfg.clone(),1,now).snapshot();
 assert!(CharacterSession::restore(cfg,&snap,now).is_ok());
}
#[test]
fn unknown_authoritative_field_negative_control() {
 let cfg=SessionConfig::default();let now="2026-09-06T00:00:00Z".parse().unwrap();
 let mut snap=CharacterSession::new(cfg.clone(),1,now).snapshot();
 snap.state["unknownProbeField"]=Value::Bool(true);snap.hash=state_hash(&snap.state);
 assert!(CharacterSession::restore(cfg,&snap,now).is_err());
}
#[test]
fn independent_hash_consistent_raw_null_must_not_restore() {
 let cfg=SessionConfig::default();let now="2026-09-06T00:00:00Z".parse().unwrap();
 let mut snap=CharacterSession::new(cfg.clone(),1,now).snapshot();
 snap.state["lastInteraction"]=Value::Null;snap.hash=state_hash(&snap.state);
 let before=snap.state.clone();assert_eq!(state_hash(&snap.state),snap.hash);
 let restored=CharacterSession::restore(cfg,&snap,now);
 match &restored { Ok(s)=>println!("RESTORED_INVALID_NULL input_hash={} output_hash={} input_has_null={} output_has_field={}",snap.hash,s.snapshot().hash,snap.state["lastInteraction"].is_null(),s.snapshot().state.get("lastInteraction").is_some()),Err(e)=>println!("REJECTED_INVALID_NULL {e:?}") }
 assert_eq!(before,snap.state,"restore must not mutate the input snapshot");
 assert!(restored.is_err(),"raw explicit null was accepted by the authoritative production restore path");
}
