//! Peer message decoding and `ValidateBasic` of every reactor: the first
//! byte picks the decoder, the rest is the message (what arrives on a
//! channel after MConnection reassembly).
#![no_main]

use celestia_core_proto::tendermint::{blocksync as bcproto, p2p as p2ppb};
use libfuzzer_sys::fuzz_target;
use prost::Message as _;

fuzz_target!(|data: &[u8]| {
    let Some((&which, msg)) = data.split_first() else {
        return;
    };
    match which % 7 {
        0 => {
            if let Ok(m) = consensus::ConsensusMessage::decode(msg) {
                let _ = m.validate_basic();
            }
        }
        1 => {
            let _ = propagation::types::Message::decode(msg);
        }
        2 => {
            if let Ok(m) = statesync::messages::Msg::decode(msg) {
                let _ = statesync::messages::validate_msg(&m, 100_000);
            }
        }
        3 => {
            if blocksync::msgs::validate_block_sync_bytes(msg).is_ok()
                && let Ok(m) = bcproto::Message::decode(msg)
            {
                let _ = blocksync::msgs::validate_msg(&m);
            }
        }
        4 => {
            if let Ok(p) = p2ppb::DefaultNodeInfo::decode(msg) {
                let ni = p2p::NodeInfo::from_proto(&p);
                let _ = ni.validate();
                let _ = ni.net_address();
                let _ = ni.compatible_with(&ni);
            }
        }
        5 => {
            if let Ok(s) = std::str::from_utf8(msg) {
                let _ = p2p::NetAddress::parse(s);
            }
        }
        _ => {
            if let Ok(p) = p2ppb::Message::decode(msg)
                && let Some(p2ppb::message::Sum::PexAddrs(a)) = p.sum
            {
                for addr in &a.addrs {
                    let _ = p2p::netaddress::NetAddress::from_proto(addr);
                }
            }
        }
    }
});
