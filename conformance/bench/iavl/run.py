"""Compare cleanly closed IAVL stores for identical writes and retained versions."""
import argparse
import hashlib
from pathlib import Path
import random
import struct
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def write_trace(path, heights):
    rng = random.Random(49152)
    with path.open("wb") as out:
        for height in range(1, heights + 1):
            accounts = (range(20000) if height == 1 else
                        [rng.randrange(20000 + height * 50) for _ in range(500)])
            out.write(struct.pack(">I", len(accounts)))
            for account in accounts:
                out.write(hashlib.sha256(struct.pack(">I", account)).digest()[:20])
                out.write(hashlib.shake_256(struct.pack(">II", height, account)).digest(80))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("celestia_app", type=Path, help="Go celestia-app checkout")
    parser.add_argument("--heights", type=int, default=200)
    parser.add_argument("--go-backend", choices=("pebbledb", "goleveldb"), default="pebbledb")
    args = parser.parse_args()
    if args.heights < 1:
        parser.error("heights must be positive")

    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        trace = work / "trace"
        write_trace(trace, args.heights)
        go = work / "go-bench"
        subprocess.run(["go", "build", "-o", go, HERE / "main.go"],
                       cwd=args.celestia_app, check=True)
        rust = ["cargo", "run", "--quiet", "--locked", "--release", "-p", "multistore",
                "--features", "rocksdb", "--example", "iavl_disk", "--"]
        roots = {}
        sizes = {}
        for name, driver in [("Go", [go, args.go_backend]), ("Rust", rust)]:
            db = work / name
            db.mkdir()
            with trace.open("rb") as stream:
                roots[name] = subprocess.check_output([*driver, db], cwd=ROOT, stdin=stream)
            sizes[name] = int(subprocess.check_output(["du", "-sk", db]).split()[0])
        if roots["Go"] != roots["Rust"] or len(roots["Go"].splitlines()) != args.heights:
            raise SystemExit("IAVL roots differ or replay ended early")
        print(f'{args.heights} matching roots. Go ({args.go_backend}) {sizes["Go"]} KiB, Rust {sizes["Rust"]} KiB, '
              f'Rust/Go {sizes["Rust"] / sizes["Go"]:.3f}')


if __name__ == "__main__":
    main()
