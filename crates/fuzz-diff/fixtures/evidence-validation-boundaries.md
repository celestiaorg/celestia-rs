# MsgSubmitEvidence validation fixtures

This exact signed fixture requires the five finalized `evidence/3` outcomes
reachable before Celestia App's nil evidence router: an omitted evidence
value, then an `Equivocation` with zero time, zero height, zero power, and an
empty consensus address. The same protobuf messages and expected results run against
the pinned v10 and v9 apps.

A wrong evidence type URL (including an empty one) and a malformed encoded `Equivocation` fail
interface unpacking during transaction decode, before the message handler.
The result contract requires every signed transaction in order, including a distinct log
marker for each validation boundary.
