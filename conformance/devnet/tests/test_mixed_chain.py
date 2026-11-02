"""Offline regression for crash-boundary checks in lib-mixed.sh.

Run with: python3 -m unittest discover -s conformance/devnet/tests -v
"""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from urllib.parse import parse_qs, urlparse


LIB = Path(__file__).resolve().parents[1] / "lib-mixed.sh"


class MixedChainTest(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)
        self.blocks = {
            h: {
                "block_id": {"hash": f"{h:02X}"},
                "block": {
                    "header": {"height": str(h), "app_hash": f"{h + 10:02X}", "proposer_address": "AA"},
                    "evidence": {"evidence": []},
                    "last_commit": {"signatures": []},
                },
            }
            for h in range(1, 4)
        }
        self.rust_blocks = json.loads(json.dumps(self.blocks))
        self.rust_blocks = {int(h): b for h, b in self.rust_blocks.items()}
        for i, kind in enumerate(("go", "rs")):
            node = self.root / f"n{i}"
            (node / "config").mkdir(parents=True)
            (node / "kind").write_text(kind)
            (node / "config/priv_validator_key.json").write_text(json.dumps({"address": ("AA", "BB")[i]}))
            # Height 2 was persisted around SIGKILL, but its commit log was lost.
            (node / "node.log").write_text("committed block height=1 hash=01 app_hash=0C\n" if i else "")

        def handler(blocks):
            class Handler(BaseHTTPRequestHandler):
                def do_GET(self):
                    url = urlparse(self.path)
                    if url.path == "/status":
                        value = {"sync_info": {"latest_block_height": "3"}}
                    elif url.path == "/block":
                        value = blocks.get(int(parse_qs(url.query)["height"][0]))
                        if value is None:
                            self.send_error(404)
                            return
                    else:
                        self.send_error(404)
                        return
                    body = json.dumps({"result": value}).encode()
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

                def log_message(self, *args):
                    pass

            return Handler

        # The checker derives node ports as reference_port + 10 * node_index.
        for _ in range(100):
            go = ThreadingHTTPServer(("127.0.0.1", 0), handler(self.blocks))
            port = go.server_port
            try:
                rust = ThreadingHTTPServer(("127.0.0.1", port + 10), handler(self.rust_blocks))
            except (OSError, OverflowError):
                go.server_close()
                continue
            break
        else:
            self.fail("could not allocate checker ports")
        self.offset = port - 26657
        for server in (go, rust):
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.addCleanup(server.server_close)
            self.addCleanup(server.shutdown)

    def check_chain(self):
        return subprocess.run(
            ["bash", "-c", 'source "$1"; OFF=$3; check_chain "$2" 2 test',
             "test", str(LIB), str(self.root), str(self.offset)],
            capture_output=True, text=True, timeout=15,
        )

    def test_retained_unlogged_height_is_verified(self):
        result = self.check_chain()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("verified 1 unlogged heights through RPC", result.stdout)

    def test_wrong_retained_block_fails(self):
        self.rust_blocks[2]["block_id"]["hash"] = "FF"
        result = self.check_chain()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rust node 1 height 2", result.stdout)

    def test_wrong_retained_app_hash_fails(self):
        self.rust_blocks[3]["block"]["header"]["app_hash"] = "FF"
        result = self.check_chain()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rust node 1 height 2", result.stdout)

    def test_unavailable_retained_height_fails(self):
        del self.rust_blocks[2]
        result = self.check_chain()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing persisted height 2", result.stdout)

    def test_missing_following_header_fails(self):
        del self.rust_blocks[3]
        result = self.check_chain()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing persisted height 2", result.stdout)

    def test_wrong_response_height_fails(self):
        self.rust_blocks[2]["block"]["header"]["height"] = "1"
        result = self.check_chain()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected retained block height", result.stdout)


if __name__ == "__main__":
    unittest.main()
