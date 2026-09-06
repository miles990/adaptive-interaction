#!/usr/bin/env python3
"""Serial simulator receiver checks; no device/renderer verification."""
import importlib.util
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from aip_applied_receiver import Receiver, canonical, loads

class ReceiverTests(unittest.TestCase):
    def setUp(self):
        self.receiver = Receiver()
        self.state = loads('{"characterId":"字串null與🌿","mood":{"kind":"neutral","intensity":0.0},"activity":"idle","attention":{"kind":"none"},"truth":{"state":"none"},"members":[],"reducedMotion":false}')

    def envelope(self, state, revision=1, kind="snapshot", base=None):
        import hashlib
        digest = hashlib.sha256(canonical(state).encode()).hexdigest()
        receipt = dict(profile="aip.applied/1", token="a"*32, generation=1, sessionId="session.home", epoch=1, revision=revision, hash=digest, messageId="host-state")
        body = dict(kind=kind, sessionEpoch=1, revision=revision, hash=digest)
        body["state" if kind == "snapshot" else "patch"] = state
        out = dict(messageType="state", messageId="host-state", sessionId="session.home", source=dict(kind="runtime", id="host"), payload=body, stateApplied=receipt)
        if base is not None: out["baseRevision"] = base
        return out

    def test_unicode_numeric_literal_and_unknown_optional_are_preserved(self):
        self.state["extension"] = loads('{"numeric":1e-7,"text":"literal null"}')
        message = self.envelope(self.state)
        self.assertIsNotNone(self.receiver.apply(message))
        self.assertIn('1e-7', canonical(self.receiver.local["state"]))
        self.assertIn('0.0', canonical(self.receiver.local["state"]))

    def test_null_missing_wrong_type_and_bad_hash_never_apply(self):
        for state in [dict(self.state, activity=None), dict(self.state, reducedMotion="false"),
                      {k:v for k,v in self.state.items() if k != "mood"}, dict(self.state, extension=None)]:
            self.assertIsNone(self.receiver.apply(self.envelope(state)))
            self.assertIsNone(self.receiver.local)
        bad = self.envelope(self.state)
        bad["payload"]["hash"] = "0"*64
        self.assertIsNone(self.receiver.apply(bad))

    def test_invalid_merge_is_atomic_and_patch_null_only_deletes(self):
        self.assertIsNotNone(self.receiver.apply(self.envelope(self.state)))
        bad = self.envelope({"mood":None}, 2, "patch", 1)
        self.assertIsNone(self.receiver.apply(bad))
        self.assertEqual(self.receiver.local["revision"], 1)
        self.assertIn("mood", self.receiver.local["state"])

    def test_bounds_and_negative_zero(self):
        self.assertFalse(self.receiver.valid(dict(self.state, characterId="x"*2001)))
        self.assertFalse(self.receiver.valid(dict(self.state, mood=loads('{"kind":"neutral","intensity":-0.0}'))))
        nested = "leaf"
        for _ in range(9): nested = {"nested":nested}
        self.assertFalse(self.receiver.valid(dict(self.state, extension=nested)))

if __name__ == "__main__": unittest.main()
