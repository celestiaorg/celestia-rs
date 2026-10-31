#!/usr/bin/env python3
"""Static regressions for capture parsing before any oracle runtime."""

import importlib.util
import pathlib
import unittest


HERE = pathlib.Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("capture_source", HERE / "capture-source.py")
CAPTURE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(CAPTURE)


class CaptureSourceTests(unittest.TestCase):
    def test_curl_http2_status_without_reason(self) -> None:
        response = CAPTURE.parse_response(
            b"HTTP/2 200\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}"
        )
        self.assertEqual(response["kind"], "http-response")
        self.assertEqual(response["http_version"], "HTTP/2")
        self.assertEqual(response["status"], 200)
        self.assertEqual(response["reason"], "")
        self.assertEqual(response["body_text"], "{}")

    def test_split_duplicate_headers_preserves_both_values(self) -> None:
        response = CAPTURE.parse_response(
            b"HTTP/1.1 200 OK\r\nVary: Origin\r\nVary: Access-Control-Request-Method\r\n"
            b"Content-Length: 0\r\n\r\n"
        )
        self.assertEqual(
            response["stable_headers"],
            [["Vary", "Origin"], ["Vary", "Access-Control-Request-Method"]],
        )


if __name__ == "__main__":
    unittest.main()
