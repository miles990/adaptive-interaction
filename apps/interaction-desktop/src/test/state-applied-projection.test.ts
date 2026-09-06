import { describe, expect, it } from "vitest";
import { characterSyncAppliedCurrent, characterSyncDeviceLine, characterSyncProfiles, projectCharacterSession, type CharacterSyncMember } from "../statusProjection/characterSync";

describe("state-applied evidence", () => {
  it("never labels transport write success as synchronized", () => {
    const snapshot = { payload: { state: {} } };
    const member: CharacterSyncMember = { name: "裝置", remote: true, presence: "online",
      canPresent: true, degraded: false, syncProfile: "full-state" };
    const projection = projectCharacterSession(snapshot, [member], { enabled: true, failedReads: 0, revokedDevice: false, connectedButNotSynced: false });
    expect(projection.state).not.toBe("synced");
    expect(projection.tone).not.toBe("ok");
  });
});

describe("receipt projection freshness", () => {
  const hash = "a".repeat(64);
  const snapshot = { sessionId: "session.home", payload: { sessionEpoch: 2, revision: 7, hash } };
  const diagnostic = { members: [{ party: {kind: "device", id: "phone"}, stateAppliedCurrent: true,
    stateDelivery: { negotiated: true, applied: { profile: "aip.applied/1", sessionId: "session.home", epoch: 2, revision: 7, hash } } }] };
  it("accepts only the currently validated host tuple", () => {
    expect(characterSyncAppliedCurrent(diagnostic, snapshot)).toEqual({phone: true});
    expect(characterSyncAppliedCurrent(diagnostic, {...snapshot, payload: {...snapshot.payload, revision: 8}})).toEqual({phone: false});
    expect(characterSyncAppliedCurrent(diagnostic, {...snapshot, sessionId: "another-session"})).toEqual({phone: false});
    expect(characterSyncAppliedCurrent(diagnostic, {...snapshot, payload: {...snapshot.payload, hash: "b".repeat(64)}})).toEqual({phone: false});
    expect(characterSyncAppliedCurrent({}, snapshot)).toEqual({});
  });
  it("does not treat an unnegotiated or missing receipt as confirmation", () => {
    const current = structuredClone(diagnostic);
    current.members[0].stateDelivery.negotiated = false;
    expect(characterSyncAppliedCurrent(current, snapshot)).toEqual({phone: false});
    expect(characterSyncAppliedCurrent({members:[{party:{kind:"device",id:"phone"},stateAppliedCurrent:true}]}, snapshot)).toEqual({phone: false});
  });
});

// The Connect page receives status rows; it must agree with the session card.
it("uses the same current receipt for the phone card and refuses write-only status", () => {
  const hash = "a".repeat(64);
  const snapshot = { sessionId: "session.home", payload: { sessionEpoch: 1, revision: 2, hash, state: {members: [
    {party: {kind: "device", id: "phone"}, presence: "online", role: "remote-renderer", unsupportedIntents: []},
  ]} } };
  const status = {characterSessionSync: [{deviceId: "phone", stateAppliedCurrent: true, stateDelivery: {negotiated: true,
    applied: {profile: "aip.applied/1", sessionId: "session.home", epoch: 1, revision: 2, hash}}}]};
  expect(characterSyncProfiles({characterSessionSync: [{deviceId: "phone", syncProfile: "pending-full-state", syncCapability: "full-state"}]})).toEqual({phone: "full-state"});
  const applied = characterSyncAppliedCurrent(status, snapshot);
  expect(applied).toEqual({phone: true});
  expect(characterSyncDeviceLine(snapshot, "phone", "full-state", applied["phone"])).toBe("角色同步：已同步");
  expect(characterSyncDeviceLine(snapshot, "phone", "full-state")).toBe("角色同步：尚未確認套用目前狀態");
});
