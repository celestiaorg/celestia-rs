//! Deeply nested amino input on a worker-sized stack (2 MiB, what tokio
//! and its blocking pool give the query handlers): `TxDecodeAmino` of a
//! `StdTx` whose `MsgExec` nests `MsgExec` thousands of levels deep. Go
//! decodes 8000 levels (its stacks grow); the Rust codec must not abort.

#![allow(clippy::unwrap_used, clippy::panic)]

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn field(n: u64, b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    varint((n << 3) | 2, &mut out);
    varint(b.len() as u64, &mut out);
    out.extend_from_slice(b);
    out
}

/// Amino binary `StdTx{msg: [MsgExec{msgs: [MsgExec{... MsgSend}]}]}`.
fn nested_exec(depth: usize) -> Vec<u8> {
    let addr = b"celestia1t9pgpk8p3ys7z35c7xpjuymexmjls7cnaf73mm";
    let mut inner = hex::decode("a8a3619a").unwrap();
    inner.extend(field(1, addr));
    inner.extend(field(2, addr));
    for _ in 0..depth {
        let mut m = hex::decode("3ec4b74a").unwrap();
        m.extend(field(1, addr));
        m.extend(field(2, &inner));
        inner = m;
    }
    let mut tx = hex::decode("282816a9").unwrap();
    tx.extend(field(1, &inner));
    tx
}

fn on_worker_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nested_msg_exec_binary() {
    on_worker_stack(|| {
        let out = legacy_amino::tx_decode_amino(&nested_exec(3)).unwrap();
        assert_eq!(out.matches("cosmos-sdk/MsgExec").count(), 3);
        let out = legacy_amino::tx_decode_amino(&nested_exec(8000)).unwrap();
        assert_eq!(out.matches("cosmos-sdk/MsgExec").count(), 8000);
    });
}

#[test]
fn nested_msg_exec_json() {
    on_worker_stack(|| {
        // Each MsgExec level is 4 JSON levels: 2000 stay below Go's 10 000.
        let json = legacy_amino::tx_decode_amino(&nested_exec(2000)).unwrap();
        let bin = legacy_amino::tx_encode_amino(json.as_bytes()).unwrap();
        assert_eq!(bin, nested_exec(2000));
        let json = legacy_amino::tx_decode_amino(&nested_exec(8000)).unwrap();
        assert_eq!(
            legacy_amino::tx_encode_amino(json.as_bytes())
                .err()
                .map(|e| e.to_string()),
            Some(
                "cannot parse disfix JSON wrapper: invalid character '{' exceeded max depth".into()
            )
        );
    });
}
