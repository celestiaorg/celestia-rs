"""Port availability across a local ABCI binary restart."""
import errno
import importlib.util
from pathlib import Path
import socket
import unittest


class UpgradeProbeTest(unittest.TestCase):
    def test_port_can_restart_after_server_closes_connection(self):
        path = Path(__file__).resolve().parents[1] / "probe-upgrade-snapshot.py"
        spec = importlib.util.spec_from_file_location("snapshot_probe", path)
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)

        with socket.socket() as server:
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            server.bind(("127.0.0.1", 0))
            probe.ADDR = server.getsockname()
            server.listen()
            with self.assertRaises(OSError) as busy:
                probe.check_port()
            self.assertEqual(busy.exception.errno, errno.EADDRINUSE)

            with socket.create_connection(probe.ADDR, timeout=5) as client:
                connection, _ = server.accept()
                connection.close()
                self.assertEqual(client.recv(1), b"")

        probe.check_port()
