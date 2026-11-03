"""Transaction failures must not be mislabeled as first-message handler errors."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "coverage-table.py"


class HandlerErrorReportTests(unittest.TestCase):
    def report(self, files):
        with tempfile.TemporaryDirectory() as folder:
            for name, text in files.items():
                (Path(folder) / (name + ".out")).write_text(text)
            return subprocess.run([sys.executable, str(SCRIPT), folder], text=True, capture_output=True)

    def rows(self, text):
        return {parts[0]: parts for line in text.splitlines() if line.startswith("| `/")
                for parts in [[p.strip().strip("`") for p in line.split("|")[1:-1]]]}

    def test_later_handler_error_is_distinct_from_first_message_tx_failure(self):
        result = self.report({"new": """TYPE /bank.Send delivered=3 go_ok=0 check_rejected=0
TYPE /staking.Delegate delivered=1 go_ok=0 check_rejected=0
ERROR_ACCOUNTING sdk-message-index-v1
CODE 1 /bank.Send staking/2 detail
CODE 2 /bank.Send sdk/4 detail
HANDLER_ERROR 1 /staking.Delegate staking/2
UNATTRIBUTED_ERROR 2 sdk/4
"""})
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = self.rows(result.stdout)
        self.assertEqual(rows["/bank.Send"][4:6], ["sdk/4: 2, staking/2: 1", "none seen"])
        self.assertEqual(rows["/staking.Delegate"][4:6], ["", "staking/2: 1"])
        self.assertIn("sdk/4: 2", result.stdout)
        self.assertIn("1/1 delivery outputs", result.stdout)

    def test_older_and_mixed_outputs_do_not_become_zero_handler_coverage(self):
        old = "TYPE /bank.Send delivered=1 go_ok=0 check_rejected=0\nCODE 1 /bank.Send sdk/11 detail\n"
        result = self.report({"old": old})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.rows(result.stdout)["/bank.Send"][5], "not measured")
        result = self.report({"old": old, "new": "ERROR_ACCOUNTING sdk-message-index-v1\nTYPE /gov.Deposit delivered=1 go_ok=1 check_rejected=0\n"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1/2 delivery outputs", result.stdout)
        self.assertEqual(self.rows(result.stdout)["/bank.Send"][5], "not measured")
        self.assertEqual(self.rows(result.stdout)["/gov.Deposit"][5], "none seen")

    def test_truncated_or_inconsistent_new_error_accounting_fails(self):
        for code in ["CODE 2 /bank.Send sdk/4 detail\n", "CODE 2\n"]:
            for counters in ["", "UNATTRIBUTED_ERROR 1 sdk/4\n",
                             "HANDLER_ERROR 3 /bank.Send sdk/4\n"]:
                with self.subTest(code=code, counters=counters):
                    result = self.report({"partial": code + "ERROR_ACCOUNTING sdk-message-index-v1\n" + counters})
                    self.assertNotEqual(result.returncode, 0)

    def test_unversioned_unknown_duplicate_or_negative_counters_fail(self):
        for text in ["HANDLER_ERROR 1 /bank.Send sdk/1\n",
                     "ERROR_ACCOUNTING future-version\n",
                     "ERROR_ACCOUNTING sdk-message-index-v1\n" * 2,
                     "ERROR_ACCOUNTING sdk-message-index-v1\nHANDLER_ERROR -1 /bank.Send sdk/1\n",
                     "ERROR_ACCOUNTING sdk-message-index-v1\nUNATTRIBUTED_ERROR -1 sdk/1\n"]:
            with self.subTest(text=text):
                result = self.report({"bad": text})
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
