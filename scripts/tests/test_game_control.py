import importlib.util
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest

SPEC = importlib.util.spec_from_file_location("game_control", Path(__file__).parents[1] / "game-control.py")
CONTROL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTROL)


class Peer:
    def __init__(self, wrong_session=False):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.port = self.listener.getsockname()[1]
        self.requests = []
        self.wrong_session = wrong_session
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()

    def serve(self):
        with self.listener, self.listener.accept()[0] as conn:
            stream = conn.makefile("rb")
            while raw := stream.readline():
                request = json.loads(raw)
                self.requests.append(request)
                reply = dict(ack=request["ack"], success=True, data=dict(
                    session="wrong" if self.wrong_session else "b" * 32,
                    command=0, completed=0, busy=False))
                payload = json.dumps(reply).encode() + b"\n"
                conn.sendall(payload[:7])
                conn.sendall(payload[7:])
            stream.close()

    def session(self):
        return dict(port=self.port, token="a" * 64, session_id="b" * 32)


class GameControlTest(unittest.TestCase):
    def test_fragmented_replies_and_secret_redaction(self):
        peer = Peer()
        client = CONTROL.Client(peer.session())
        try:
            result = client.run("state")
        finally:
            client.close()
        peer.thread.join(2)
        self.assertNotIn("a" * 64, json.dumps(result))
        self.assertNotIn("session", result["after"])
        self.assertTrue(all(r["token"] == "a" * 64 for r in peer.requests))
        self.assertEqual([r["ack"] for r in peer.requests], [1, 2, 3])

    def test_wrong_session_is_rejected_before_action(self):
        peer = Peer(wrong_session=True)
        with self.assertRaisesRegex(RuntimeError, "wrong game session"):
            CONTROL.Client(peer.session())
        peer.thread.join(2)
        self.assertEqual([r["op"] for r in peer.requests], ["state"])

    def test_descriptor_cannot_be_moved_to_other_work_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session.json"
            path.write_text(json.dumps(dict(format="KFXCONTROL1", token="a" * 64,
                                           session_id="b" * 32, port=5599, work="/elsewhere")))
            with self.assertRaisesRegex(ValueError, "directory"):
                CONTROL.read_session(path)

    def test_invalid_token_rejected_without_connecting(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session.json"
            path.write_text(json.dumps(dict(format="KFXCONTROL1", token="short", port=5599)))
            with self.assertRaisesRegex(ValueError, "descriptor"):
                CONTROL.read_session(path)


if __name__ == "__main__":
    unittest.main()
