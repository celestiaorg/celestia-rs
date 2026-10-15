//! Regression (fuzzing, `fuzz/fuzz_targets/rpc_request.rs`): deeply nested
//! JSON-RPC bodies must not overflow a worker thread's stack. Go's
//! `encoding/json` accepts 10 000 levels; the parser, the tree's drop and
//! the id formatting must handle that depth without recursion.

#![allow(clippy::unwrap_used, clippy::panic)]

fn on_worker_stack(f: impl FnOnce() + Send + 'static) {
    // tokio's default worker stack size.
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nested_request_bodies() {
    on_worker_stack(|| {
        let deep = |d: usize| format!("{}{}", "[".repeat(d), "]".repeat(d));
        // A batch of one deeply nested (invalid) request.
        assert!(rpc::server::parse_requests(&deep(10_000)).is_err());
        assert_eq!(
            rpc::server::parse_requests(&"[".repeat(500_000)).err(),
            Some("error unmarshaling request: invalid character '[' exceeded max depth".into())
        );
        // A deeply nested id is formatted like Go's %v.
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"status"}}"#,
            deep(9_999)
        );
        let err = rpc::server::parse_requests(&body).unwrap_err();
        assert!(
            err.starts_with("error unmarshaling request: json-rpc ID ([[["),
            "{err}"
        );
        assert!(
            err.ends_with("]]]) is of unknown type ([]interface {})"),
            "{err}"
        );
        let small = r#"{"jsonrpc":"2.0","id":[1,{"b":[],"a":null}],"method":"status"}"#;
        assert_eq!(
            rpc::server::parse_requests(small).err(),
            Some(
                "error unmarshaling request: json-rpc ID ([1 map[a:<nil> b:[]]]) is of unknown type ([]interface {})"
                    .into()
            )
        );
    });
}
