//! Submit a real ICS-20 transfer through the v12 packet-forward middleware and retry it.
#![allow(clippy::unwrap_used)]
use celestia_proto::ibc::core::channel::v1::{Channel, Counterparty, Order, Packet, State};
use celestia_proto::packetforward::v1::InFlightPacket;
use prost::Message;
use sdk::{Context, ExecMode, Header, Kind, MultiStore, coins::Coin};
use x_ibc::router::IbcModule;
use x_packetforward::middleware::PacketForwardMiddleware;
use x_transfer::relay::{PacketData, TransferModule};

// A return of native tokens on the incoming channel funds the next ICS-20 transfer.
fn store() -> MultiStore {
    let mut store = MultiStore::new();
    for key in [
        "acc",
        "bank",
        "transfer",
        "ibc",
        "capability",
        x_packetforward::STORE_KEY,
    ] {
        store.mount(key, Kind::Iavl).unwrap();
    }
    store.mount("memory:capability", Kind::Memory).unwrap();
    store
}

fn open_channel(ctx: &mut Context<'_>, channel: &str) {
    use x_ibc::connection::LOCALHOST_CONNECTION_ID;

    x_ibc::client::set_params(
        ctx,
        &celestia_proto::ibc::core::client::v1::Params {
            allowed_clients: vec!["*".into()],
        },
    )
    .unwrap();
    x_ibc::client::create_localhost_client(ctx).unwrap();
    x_ibc::connection::create_sentinel_localhost_connection(ctx).unwrap();
    let path = x_ibc::host::channel_capability_path("transfer", channel);
    let cap = x_ibc::port::SCOPED_KEEPER
        .new_capability(ctx, &path)
        .unwrap();
    x_transfer::SCOPED_KEEPER
        .claim_capability(ctx, &cap, &path)
        .unwrap();
    x_ibc::channel::set_channel(
        ctx,
        "transfer",
        channel,
        &Channel {
            state: State::Open.into(),
            ordering: Order::Unordered.into(),
            counterparty: Some(Counterparty {
                port_id: "transfer".into(),
                channel_id: "channel-9".into(),
            }),
            connection_hops: vec![LOCALHOST_CONNECTION_ID.into()],
            version: "ics20-1".into(),
            ..Default::default()
        },
    )
    .unwrap();
    x_ibc::channel::set_next_sequence_send(ctx, "transfer", channel, 1).unwrap();
}

fn receive(
    ctx: &mut Context<'_>,
    middleware: &PacketForwardMiddleware<TransferModule>,
    memo: &str,
) {
    ctx.consensus_params.version =
        Some(celestia_proto::celestia_core_proto::tendermint::types::VersionParams { app: 12 });
    x_bank::set_send_enabled(ctx, "utia", true).unwrap();
    x_transfer::set_params(
        ctx,
        &celestia_proto::ibc::applications::transfer::v1::Params {
            send_enabled: true,
            receive_enabled: true,
        },
    )
    .unwrap();
    let token = Coin::new("utia", 1);
    x_bank::set_balance(
        ctx,
        &x_transfer::escrow_address("transfer", "channel-7"),
        &token,
    )
    .unwrap();
    x_transfer::set_total_escrow_for_denom(ctx, &token.to_proto()).unwrap();
    let packet = Packet {
        sequence: 1,
        source_port: "transfer".into(),
        source_channel: "channel-8".into(),
        destination_port: "transfer".into(),
        destination_channel: "channel-7".into(),
        data: PacketData {
            denom: "transfer/channel-8/utia".into(),
            amount: "1".into(),
            sender: "upstream".into(),
            receiver: "pfm".into(),
            memo: memo.into(),
        }
        .get_bytes(),
        ..Default::default()
    };
    let ack = middleware.on_recv_packet(ctx, &packet, &[]).unwrap();
    assert!(ack.is_none(), "forward returned {ack:?}");
}

fn last_sent(ctx: &Context<'_>) -> Packet {
    let event = ctx
        .events
        .events()
        .iter()
        .rev()
        .find(|e| e.r#type == "send_packet")
        .unwrap();
    let attr = |key: &str| {
        event
            .attributes
            .iter()
            .find(|a| a.key == key)
            .unwrap()
            .value
            .as_str()
    };
    Packet {
        sequence: attr("packet_sequence").parse().unwrap(),
        source_port: attr("packet_src_port").into(),
        source_channel: attr("packet_src_channel").into(),
        destination_port: attr("packet_dst_port").into(),
        destination_channel: attr("packet_dst_channel").into(),
        timeout_timestamp: attr("packet_timeout_timestamp").parse().unwrap(),
        data: attr("packet_data").as_bytes().to_vec(),
        ..Default::default()
    }
}

fn pending(ctx: &mut Context<'_>, packet: &Packet) -> InFlightPacket {
    let key = x_packetforward::refund_packet_key(
        &packet.source_channel,
        &packet.source_port,
        packet.sequence,
    );
    let bytes = ctx.get(x_packetforward::STORE_KEY, &key).unwrap().unwrap();
    InFlightPacket::decode(bytes.as_slice()).unwrap()
}

#[test]
fn forwarded_and_retried_transfers_keep_the_original_memo_bytes() {
    let mut store = store();
    let mut ctx = Context::new(
        &mut store,
        Header {
            height: 1,
            ..Default::default()
        },
        ExecMode::Finalize,
    );
    open_channel(&mut ctx, "channel-0");
    let middleware = PacketForwardMiddleware {
        app: TransferModule,
        retries_on_timeout: 1,
        forward_timeout: 500,
    };
    let next = r#"{ "b" : 1,  "a" : [ 1 , 2 ] }"#;
    let memo = format!(
        r#"{{"forward":{{"receiver":"next","port":"transfer","channel":"channel-0","next":{next}}}}}"#
    );
    receive(&mut ctx, &middleware, &memo);
    let first = last_sent(&ctx);
    assert_eq!(PacketData::decode(&first.data).unwrap().memo, next);
    assert_eq!(first.timeout_timestamp, 500);
    assert_eq!(pending(&mut ctx, &first).retries_remaining, 1);
    ctx.header.time.nanos = 500;
    middleware.on_timeout_packet(&mut ctx, &first, &[]).unwrap();
    let retry = last_sent(&ctx);
    assert_eq!(retry.sequence, first.sequence + 1);
    assert_eq!(PacketData::decode(&retry.data).unwrap().memo, next);
    assert_eq!(retry.timeout_timestamp, 1000);
    assert_eq!(pending(&mut ctx, &retry).retries_remaining, 0);
}
