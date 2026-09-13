import importlib.util
import json
import os
from pathlib import Path
import socket
import time
import unittest

SPEC = importlib.util.spec_from_file_location("game_control", Path(__file__).parents[1] / "game-control.py")
CONTROL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTROL)
SESSION = os.environ.get("KFX_CONTROL_TEST_SESSION")


@unittest.skipUnless(SESSION, "requires an isolated running game via KFX_CONTROL_TEST_SESSION")
class NativeControlIntegration(unittest.TestCase):
    def setUp(self):
        self.session = CONTROL.read_session(Path(SESSION))
        time.sleep(0.15)
        self.sock = socket.create_connection(("127.0.0.1", self.session["port"]), timeout=2)
        self.stream = self.sock.makefile("rb")
        self.ack = 0

    def tearDown(self):
        self.stream.close()
        self.sock.close()

    def request(self, authenticate=True, fragment=False, **fields):
        self.ack += 1
        packet = dict(ack=self.ack, **fields)
        if authenticate:
            packet["token"] = self.session["token"]
        payload = json.dumps(packet).encode() + b"\n"
        if fragment:
            self.sock.sendall(payload[:9])
            time.sleep(0.05)
            self.sock.sendall(payload[9:])
        else:
            self.sock.sendall(payload)
        while line := self.stream.readline():
            result = json.loads(line)
            if result.get("ack") == packet["ack"]:
                return result
        self.fail("connection closed before acknowledgement")

    def test_authentication_fragmentation_and_bounds(self):
        denied = self.request(authenticate=False, action="get_kfx_info")
        self.assertFalse(denied["success"])
        self.assertEqual(denied["error"], "CONTROL_UNAUTHORIZED")
        state = self.request(action="control", op="state", fragment=True, ignored="{quoted braces}")
        self.assertTrue(state["success"])
        self.assertEqual(state["data"]["session"], self.session["session_id"])
        invalid = self.request(action="control", op="drag", x=0, y=0, to_x=-1, to_y=0)
        self.assertFalse(invalid["success"])
        self.assertEqual(invalid["error"], "INVALID_POSITION")

    def test_overflow_drops_subscription_before_unauthenticated_reconnect(self):
        state = self.request(action="control", op="state")["data"]
        self.assertEqual(state["frontend"], 0, "run this integration test in gameplay")
        self.assertFalse(state["paused"])
        reply = self.request(action="subscribe_var", var="GAME_TURN")
        self.assertTrue(reply["success"])
        update = json.loads(self.stream.readline())
        self.assertNotIn("ack", update, "a live changing subscription must be observed first")
        self.sock.sendall(b"x" * 4096)
        try:
            while self.stream.readline():
                pass
        except ConnectionResetError:
            pass
        self.tearDown()
        self.setUp()
        denied = self.request(authenticate=False, action="get_kfx_info")
        self.assertFalse(denied["success"])
        self.sock.settimeout(0.4)
        with self.assertRaises(socket.timeout):
            self.stream.readline()
        self.tearDown()
        self.setUp()
        current = self.request(action="control", op="state")["data"]
        self.assertGreater(current["turn"], state["turn"], "game must continue generating changing turns")

    def test_busy_and_disconnect_release_held_input(self):
        started = self.request(action="control", op="key", key="Left", frames=120)["data"]
        rejected = self.request(action="control", op="click", x=1, y=1)
        self.assertFalse(rejected["success"])
        self.assertEqual(rejected["error"], "CONTROL_BUSY")
        self.tearDown()
        self.setUp()
        deadline = time.monotonic() + 2
        while True:
            state = self.request(action="control", op="state")["data"]
            if not state["busy"]:
                break
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.03)
        self.assertEqual(state["completed"], started["command"])
        self.assertEqual(state["error"], "CANCELLED")
        accepted = self.request(action="control", op="wait", frames=2)
        self.assertTrue(accepted["success"])


if __name__ == "__main__":
    unittest.main()
