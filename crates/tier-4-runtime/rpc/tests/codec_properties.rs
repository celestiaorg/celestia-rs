//! Properties of valid CometBFT RPC parameter encodings.

use base64::Engine as _;
use proptest::prelude::*;
use rpc::json::J;
use rpc::params::{Arg, ArgType, cmtjson_decode, json_args, uri_arg};
use rpc::server::{Id, RpcResponse, parse_requests};
use serde_json::json;

fn signed() -> impl Strategy<Value = i64> {
    prop_oneof![Just(i64::MIN), Just(0), Just(i64::MAX), any::<i64>()]
}

fn unsigned() -> impl Strategy<Value = u64> {
    prop_oneof![Just(0), Just(u64::MAX), any::<u64>()]
}

fn rpc_string() -> impl Strategy<Value = String> {
    let character = prop_oneof![
        any::<char>(),
        prop::sample::select(vec![
            '"', '\\', '\n', '\0', '<', '>', '&', '\u{2028}', 'é', '💫'
        ]),
    ];
    prop::collection::vec(character, 0..64).prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    #[test]
    fn named_and_positional_params_preserve_values(
        height in prop::option::of(signed()),
        limit in unsigned(),
        query in rpc_string(),
        prove in any::<bool>(),
    ) {
        let names = ["height", "limit", "query", "prove"];
        let types = [ArgType::I64Ptr, ArgType::U64, ArgType::Str, ArgType::Bool];
        let encoded_height = height.map_or(J::Null, J::i64s);
        let encoded_limit = J::u64s(limit);
        // The writer accepts Go-form strings, which reserve some Unicode scalars.
        let encoded_query = J::str(gostr::from_go_bytes(query.as_bytes()));
        let encoded_prove = J::Bool(prove);

        let mut named = J::obj([
            ("query", encoded_query.clone()),
            ("prove", encoded_prove.clone()),
            ("limit", encoded_limit.clone()),
        ]);
        if height.is_some() {
            named.push("height", encoded_height.clone());
        }
        let positional = J::Arr(vec![encoded_height, encoded_limit, encoded_query, encoded_prove]);

        let expected = vec![
            height.map_or(Arg::Null, |value| Arg::Int(i128::from(value))),
            Arg::Int(i128::from(limit)),
            Arg::Str(query),
            Arg::Bool(prove),
        ];
        prop_assert_eq!(json_args(&names, &types, &named.encode()), Ok(expected.clone()));
        prop_assert_eq!(json_args(&names, &types, &positional.encode()), Ok(expected));
    }

    #[test]
    fn byte_params_preserve_contents_across_wire_encodings(
        bytes in prop::collection::vec(any::<u8>(), 1..129),
    ) {
        let b64 = J::b64(&bytes).encode();
        let hex = J::hex(&bytes).encode();
        let uri = format!("0x{}", hex::encode(&bytes));
        let expected = Arg::Bytes(bytes);

        prop_assert_eq!(cmtjson_decode(&b64, ArgType::Bytes), Ok(expected.clone()));
        prop_assert_eq!(cmtjson_decode(&hex, ArgType::HexBytes), Ok(expected.clone()));
        prop_assert_eq!(uri_arg(ArgType::Bytes, &uri), Ok(expected));
    }

    #[test]
    fn byte_list_params_preserve_order_and_empty_elements(
        entries in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..33), 0..9),
    ) {
        let encoded = J::Arr(entries.iter().map(|entry| J::b64n(entry)).collect()).encode();
        let expected = if entries.is_empty() { Arg::Null } else { Arg::BytesList(entries) };

        prop_assert_eq!(cmtjson_decode(&encoded, ArgType::BytesList), Ok(expected));
    }

    #[test]
    fn responses_echo_unicode_request_ids(
        prefix in rpc_string(),
        private_use in prop::char::range('\u{10ff80}', '\u{10fffd}'),
        suffix in rpc_string(),
    ) {
        let id = format!("{prefix}{private_use}{suffix}");
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": "status" }).to_string();
        let requests = parse_requests(&body).unwrap();
        let [request] = requests.as_slice() else {
            return Err(TestCaseError::fail("expected one request"));
        };
        for response in [
            RpcResponse::ok(request.id.clone(), &J::Null),
            RpcResponse::method_not_found(request.id.clone()),
        ] {
            let decoded: serde_json::Value = serde_json::from_str(&response.encode()).unwrap();
            prop_assert_eq!(decoded["id"].as_str(), Some(id.as_str()));
        }
    }

    #[test]
    fn batched_transaction_requests_keep_ids_and_payloads_together(
        entries in prop::collection::vec((
            prop::option::of(any::<i32>()),
            any::<bool>(),
            prop::collection::vec(any::<u8>(), 1..65),
        ), 1..9),
    ) {
        let requests = entries.iter().map(|(id, named, bytes)| {
            let tx = base64::engine::general_purpose::STANDARD.encode(bytes);
            let params = if *named { json!({ "tx": tx }) } else { json!([tx]) };
            let mut request = json!({ "jsonrpc": "2.0", "method": "check_tx", "params": params });
            if let Some(id) = id {
                request["id"] = json!(id);
            }
            request
        }).collect::<Vec<_>>();
        let batch = serde_json::to_string(&requests).unwrap();
        let parsed = parse_requests(&batch).unwrap();

        prop_assert_eq!(parsed.len(), entries.len());
        for (request, (id, _, bytes)) in parsed.into_iter().zip(entries) {
            prop_assert_eq!(request.id, id.map(|n| Id::Int(i64::from(n))));
            if id.is_some() {
                prop_assert_eq!(request.method, "check_tx");
                prop_assert_eq!(
                    json_args(&["tx"], &[ArgType::Bytes], &request.params),
                    Ok(vec![Arg::Bytes(bytes)]),
                );
            } else {
                // Go leaves the request empty for a notification.
                prop_assert!(request.method.is_empty() && request.params.is_empty());
            }
        }
    }
}
