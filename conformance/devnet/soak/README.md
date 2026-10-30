# Soak client failures

The bank workload records exceptions during account initialization, broadcasts,
and sequence resynchronization as `client-error` entries in `events.log`.
Each entry retains the exception type, bounded message and traceback, operation,
node kind and endpoint, chain height, and public account context. Broadcast
failures also retain the transaction SHA-256, byte length, sequence, account
number, and request timeout. Private keys and raw signed transactions are not
recorded.

`report.json`, `report.md`, and restart state retain counts and the first/latest
samples per category/operation in `metrics.client_errors`. The grouping excludes
exception messages and transaction identifiers to bound memory and report size.
The event log retains every occurrence; its disk usage grows with the run like
the existing node and workload logs. The existing `bank.errors` count still
counts broadcast exceptions only. Account-query exceptions have separate counts.
This also works when resuming older state without a `client_errors` entry.

A recorded client exception needs investigation: a timeout or invalid response
alone does not attribute a fault to either implementation. These diagnostics do
not make an earlier unclassified failure explainable, and do not establish an
all-API soak result for tracking issue #192.

Run the failure-injection regressions with Python 3.10+ and `cryptography`:

```sh
python3 -m unittest discover -s conformance/devnet/soak -p 'test_*.py' -v
```

One test serves malformed JSON through a real local HTTP socket to the independent
Cosmos transaction client and verifies the actual workload records the failure
and request context. The other exercises repeated large exception messages to
check that persisted summaries stay bounded.
