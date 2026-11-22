"""Loopback TCP link used to partition a non-validating peer in the private fixture."""
import asyncio
from contextlib import suppress
from pathlib import Path
import sys


async def copy(source, destination):
    while chunk := await source.read(65536):
        destination.write(chunk)
        await destination.drain()
    destination.write_eof()


async def forward(reader, writer):
    try:
        upstream, outgoing = await asyncio.open_connection("127.0.0.1", 36656)
        try:
            async with asyncio.TaskGroup() as group:
                group.create_task(copy(reader, outgoing))
                group.create_task(copy(upstream, writer))
        finally:
            outgoing.close()
    finally:
        writer.close()


async def main():
    server = await asyncio.start_server(forward, "127.0.0.1", 38656)
    async with server:
        Path(sys.argv[1]).touch()
        await server.serve_forever()


if __name__ == "__main__":
    with suppress(KeyboardInterrupt):
        asyncio.run(main())
