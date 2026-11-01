"""Issue82 storage control: different SQLite build; never a product/gate metric."""

import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import time


def main():
    fixture = Path(tempfile.mkdtemp(prefix="meshloop_issue82_control_"))
    print(f"fixture={fixture}", file=sys.stderr, flush=True)
    connection = None
    samples = []
    output = {
        "schema": "issue82-storage-control-v1",
        "interpretation": "Different SQLite library; storage/environment control only, not canonical gate or product latency",
        "fixture": str(fixture), "temp_root": tempfile.gettempdir(),
        "temp_volume": os.path.splitdrive(str(fixture.resolve()))[0],
        "sqlite_library_version": sqlite3.sqlite_version,
        "python_version": sys.version, "python_executable": sys.executable,
        "status": "partial", "ordered_samples_ms": samples,
    }
    try:
        connection = sqlite3.connect(fixture / "control.sqlite", isolation_level=None)
        connection.execute("PRAGMA journal_mode=WAL")
        connection.execute("PRAGMA synchronous=FULL")
        pragmas = {
            name: connection.execute(f"PRAGMA {name}").fetchone()[0]
            for name in ("journal_mode", "synchronous", "page_size", "wal_autocheckpoint")
        }
        output["actual_pragmas"] = pragmas
        output["sqlite_compile_options"] = [row[0] for row in connection.execute("PRAGMA compile_options")]
        if pragmas["journal_mode"].lower() != "wal" or pragmas["synchronous"] != 2:
            raise RuntimeError(f"WAL/FULL not established: {pragmas}")
        connection.execute("CREATE TABLE control (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        for index in range(50):
            started = time.perf_counter_ns()
            connection.execute("BEGIN IMMEDIATE")
            connection.execute("INSERT INTO control VALUES (?, ?)", (index, "issue82-control"))
            connection.execute("COMMIT")
            samples.append((time.perf_counter_ns() - started) / 1_000_000)
        output.update(status="completed", p95_index_zero_based=47, p95_ms=sorted(samples)[47])
    except Exception as error:
        output["error"] = repr(error)
        raise
    finally:
        output["sample_count"] = len(samples)
        output["transaction_open_at_end"] = connection.in_transaction if connection else None
        print(json.dumps(output), flush=True)
        if connection:
            connection.close()


if __name__ == "__main__":
    main()
