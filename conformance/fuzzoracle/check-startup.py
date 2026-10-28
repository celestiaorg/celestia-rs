#!/usr/bin/env python3
"""Exercise isolated Go-oracle startup, EOF cleanup and Rust-driven restarts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile


def check(binary, driver, version):
    with tempfile.TemporaryDirectory(prefix="oracle-startup-") as folder:
        root = Path(folder)
        # Do not alter HOME: the real operator home must remain untouched.
        env = {**os.environ, "TMPDIR": folder, "FUZZ_ORACLE_TMPDIR": folder,
               "GOMAXPROCS": "1", "GOMEMLIMIT": "512MiB"}
        children = []
        with tempfile.TemporaryFile() as errors:
            try:
                for _ in range(2):
                    child = subprocess.Popen([str(binary)], env=env, stdin=subprocess.PIPE,
                                             stdout=subprocess.PIPE, stderr=errors)
                    children.append(child)
                    child.stdin.write(b"tx 00\n")
                    child.stdin.flush()
                    with selectors.DefaultSelector() as ready:
                        ready.register(child.stdout, selectors.EVENT_READ)
                        if not ready.select(timeout=30):
                            raise RuntimeError("oracle startup response timed out")
                    response = child.stdout.readline()
                    if not response or json.loads(response).get("ok") is not False:
                        raise RuntimeError(f"missing expected decoding rejection: {response!r}")
                homes = list(root.iterdir())
                if len(homes) != 2 or any(not p.name.startswith("celestia-fuzzoracle-")
                                          or not p.is_dir() for p in homes):
                    raise RuntimeError(f"oracles did not create separate homes: {homes}")
                if any(not (p / "data").is_dir() for p in homes):
                    raise RuntimeError("App startup did not use each private home/data directory")
                for child in children:
                    child.stdin.close()
                    if child.wait(timeout=30) != 0:
                        raise RuntimeError("oracle failed on EOF")
                if list(root.iterdir()):
                    raise RuntimeError("standalone oracle homes survived normal exit")
            except Exception as error:
                errors.seek(0)
                raise RuntimeError(f"{binary}: {error}\n{errors.read().decode(errors='replace')}") from error
            finally:
                for child in children:
                    if child.poll() is None:
                        child.kill()
                    child.wait()
                    if not child.stdin.closed:
                        child.stdin.close()
                    child.stdout.close()
        if driver:
            env[f"FUZZ_ORACLE_V{version}"] = str(binary)
            # A real request/reply and Oracle::drop, repeated in fresh drivers.
            # The driver kills Go; its parent-owned scratch must still vanish.
            for _ in range(2):
                result = subprocess.run(
                    [str(driver), "record", str(version), "oracle-startup-smoke"],
                    input="iw 00\n",
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=60,
                )
                if (
                    result.returncode
                    or "agree iw" not in result.stderr
                    or "MISMATCH" in result.stderr
                ):
                    raise RuntimeError(
                        f"Rust/Go smoke failed: {result.stdout}\n{result.stderr}"
                    )
                record = json.loads(result.stdout)
                if (
                    record.get("app_version") != version
                    or record.get("op") != "iw"
                    or record.get("input") != "00"
                    or record["go"].get("err")
                    or record["go"].get("is_wrapper", False)
                ):
                    raise RuntimeError(f"missing expected index-wrapper reply: {record}")
                if list(root.iterdir()):
                    raise RuntimeError("Rust-driven oracle scratch survived forced shutdown")
        print(json.dumps({"binary": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                          "version": version, "standalone_instances": 2,
                          "rust_drivers": 2 if driver else 0, "status": "passed"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--version", type=int, choices=[9, 10], required=True)
    parser.add_argument("--driver", type=Path)
    args = parser.parse_args()
    check(args.binary.resolve(), args.driver.resolve() if args.driver else None, args.version)


if __name__ == "__main__":
    main()
