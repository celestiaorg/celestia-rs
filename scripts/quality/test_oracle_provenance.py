"""Oracle provenance receipts bind workflow evidence to exact binary bytes."""

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
SPEC = importlib.util.spec_from_file_location(
    "oracle_provenance", Path(__file__).with_name("oracle_provenance.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
sys.path.pop(0)


class OracleProvenanceTest(unittest.TestCase):
    def test_records_digest_size_and_protobuf_seed(self):
        with tempfile.TemporaryDirectory() as folder:
            binary = Path(folder) / "oracle"
            binary.write_bytes(bytes(range(256)) * 4)
            receipt = MODULE.provenance([f"v10={binary}"])
            self.assertEqual(receipt["schema_version"], 1)
            self.assertEqual(receipt["binaries"]["v10"]["size"], 1024)
            self.assertEqual(
                receipt["binaries"]["v10"]["sha256"],
                "785b0751fc2c53dc14a4ce3d800e69ef9ce1009eb327ccf458afe09c242c26c9",
            )
            self.assertEqual(
                receipt["binaries"]["v10"]["protobuf_go_detrand"],
                {
                    "seed_hex": "0x0ce7a0bfde0748e1",
                    "separator": "nonbreaking-space",
                    "separator_utf8_hex": "c2a0",
                },
            )

    def test_rejects_malformed_and_duplicate_labels(self):
        with tempfile.TemporaryDirectory() as folder:
            binary = Path(folder) / "oracle"
            binary.write_bytes(b"binary")
            with self.assertRaisesRegex(ValueError, "LABEL=PATH"):
                MODULE.provenance([str(binary)])
            with self.assertRaisesRegex(ValueError, "duplicate"):
                MODULE.provenance([f"v10={binary}", f"v10={binary}"])


if __name__ == "__main__":
    unittest.main()
