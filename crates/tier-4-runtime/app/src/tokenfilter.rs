//! celestia-app v4.1.0 and v5.0.12 `x/tokenfilter`: the outermost IBC
//! middleware of the transfer stack. `OnRecvPacket` accepts only a token
//! returning to this chain (its denom trace starts with the packet's source
//! port and channel); any other ICS-20 packet gets an error
//! acknowledgement. Every other callback is the wrapped module's. The
//! middleware embeds only `porttypes.IBCModule`, so the transfer route does
//! not implement `UpgradableModule` while it is active. v6.4.10 removed it
//! ([`sdk::version::Release::has_token_filter`]).

use celestia_proto::capability::v1::Capability;
use celestia_proto::ibc::core::channel::v1::{Counterparty, Order, Packet};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_ibc::router::{Acknowledgement, IbcModule};
use x_transfer::relay as tr;

/// The middleware's module name (`tokenfilter.ModuleName`).
pub const MODULE_NAME: &str = "tokenfilter";

/// `tokenFilterMiddleware` around `M`, active where the executing release
/// has it.
#[derive(Debug, Clone)]
pub struct TokenFilter<M>(pub M);

fn active(ctx: &Context<'_>) -> bool {
    ctx.release().has_token_filter()
}

impl<M: IbcModule> IbcModule for TokenFilter<M> {
    fn on_chan_open_init(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        version: &str,
    ) -> Result<String> {
        self.0.on_chan_open_init(
            ctx,
            order,
            connection_hops,
            port_id,
            channel_id,
            cap,
            counterparty,
            version,
        )
    }

    fn on_chan_open_try(
        &self,
        ctx: &mut Context<'_>,
        order: Order,
        connection_hops: &[String],
        port_id: &str,
        channel_id: &str,
        cap: &Capability,
        counterparty: &Counterparty,
        counterparty_version: &str,
    ) -> Result<String> {
        self.0.on_chan_open_try(
            ctx,
            order,
            connection_hops,
            port_id,
            channel_id,
            cap,
            counterparty,
            counterparty_version,
        )
    }

    fn on_chan_open_ack(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        counterparty_channel_id: &str,
        counterparty_version: &str,
    ) -> Result<()> {
        self.0.on_chan_open_ack(
            ctx,
            port_id,
            channel_id,
            counterparty_channel_id,
            counterparty_version,
        )
    }

    fn on_chan_open_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.0.on_chan_open_confirm(ctx, port_id, channel_id)
    }

    fn on_chan_close_init(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.0.on_chan_close_init(ctx, port_id, channel_id)
    }

    fn on_chan_close_confirm(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
    ) -> Result<()> {
        self.0.on_chan_close_confirm(ctx, port_id, channel_id)
    }

    fn on_recv_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        relayer: &[u8],
    ) -> Result<Option<Acknowledgement>> {
        if !active(ctx) {
            return self.0.on_recv_packet(ctx, packet, relayer);
        }
        // An undecodable packet goes down the stack.
        let Ok(data) = tr::PacketData::decode(&packet.data) else {
            return self.0.on_recv_packet(ctx, packet, relayer);
        };
        if tr::receiver_chain_is_source(&packet.source_port, &packet.source_channel, &data.denom) {
            return self.0.on_recv_packet(ctx, packet, relayer);
        }
        let err = errs::invalid_type(format!(
            "only native denom transfers accepted, got {}",
            data.denom
        ));
        ctx.events.emit(event(
            "fungible_token_packet",
            [
                ("module", MODULE_NAME.to_owned()),
                ("sender", data.sender.clone()),
                ("receiver", data.receiver.clone()),
                ("denom", data.denom.clone()),
                ("amount", data.amount.clone()),
                ("memo", data.memo.clone()),
                ("success", "false".to_owned()),
                ("error", err.log.clone()),
            ],
        ));
        Ok(Some(Acknowledgement::error(&err)))
    }

    fn on_acknowledgement_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        acknowledgement: &[u8],
        relayer: &[u8],
    ) -> Result<()> {
        self.0
            .on_acknowledgement_packet(ctx, packet, acknowledgement, relayer)
    }

    fn on_timeout_packet(
        &self,
        ctx: &mut Context<'_>,
        packet: &Packet,
        relayer: &[u8],
    ) -> Result<()> {
        self.0.on_timeout_packet(ctx, packet, relayer)
    }

    fn upgradable(&self) -> bool {
        !sdk::version::release(0).has_token_filter() && self.0.upgradable()
    }

    fn on_chan_upgrade_init(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        order: Order,
        connection_hops: &[String],
        version: &str,
    ) -> Result<String> {
        self.0
            .on_chan_upgrade_init(ctx, port_id, channel_id, order, connection_hops, version)
    }

    fn on_chan_upgrade_try(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        order: Order,
        connection_hops: &[String],
        counterparty_version: &str,
    ) -> Result<String> {
        self.0.on_chan_upgrade_try(
            ctx,
            port_id,
            channel_id,
            order,
            connection_hops,
            counterparty_version,
        )
    }

    fn on_chan_upgrade_ack(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        counterparty_version: &str,
    ) -> Result<()> {
        self.0
            .on_chan_upgrade_ack(ctx, port_id, channel_id, counterparty_version)
    }

    fn on_chan_upgrade_open(
        &self,
        ctx: &mut Context<'_>,
        port_id: &str,
        channel_id: &str,
        order: Order,
        connection_hops: &[String],
        version: &str,
    ) {
        self.0
            .on_chan_upgrade_open(ctx, port_id, channel_id, order, connection_hops, version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
    use sdk::{ExecMode, Header, MultiStore};

    /// The wrapped module: acknowledges every packet with `[1]`.
    struct Accept;

    impl IbcModule for Accept {
        fn on_chan_open_init(
            &self,
            _: &mut Context<'_>,
            _: Order,
            _: &[String],
            _: &str,
            _: &str,
            _: &Capability,
            _: &Counterparty,
            v: &str,
        ) -> Result<String> {
            Ok(v.to_owned())
        }
        fn on_chan_open_try(
            &self,
            _: &mut Context<'_>,
            _: Order,
            _: &[String],
            _: &str,
            _: &str,
            _: &Capability,
            _: &Counterparty,
            v: &str,
        ) -> Result<String> {
            Ok(v.to_owned())
        }
        fn on_chan_open_ack(
            &self,
            _: &mut Context<'_>,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<()> {
            Ok(())
        }
        fn on_chan_open_confirm(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn on_chan_close_init(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn on_chan_close_confirm(&self, _: &mut Context<'_>, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn on_recv_packet(
            &self,
            _: &mut Context<'_>,
            _: &Packet,
            _: &[u8],
        ) -> Result<Option<Acknowledgement>> {
            Ok(Some(Acknowledgement::result(&[1])))
        }
        fn on_acknowledgement_packet(
            &self,
            _: &mut Context<'_>,
            _: &Packet,
            _: &[u8],
            _: &[u8],
        ) -> Result<()> {
            Ok(())
        }
        fn on_timeout_packet(&self, _: &mut Context<'_>, _: &Packet, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn upgradable(&self) -> bool {
            true
        }
    }

    fn packet(denom: &str) -> Packet {
        Packet {
            source_port: "transfer".into(),
            source_channel: "channel-2".into(),
            data: format!(r#"{{"amount":"5","denom":"{denom}","receiver":"r","sender":"s"}}"#)
                .into_bytes(),
            ..Packet::default()
        }
    }

    /// App versions 4 and 5 reject a foreign denom with an error
    /// acknowledgement and the tokenfilter event, and pass a returning
    /// one; later versions pass both.
    #[test]
    fn filters_foreign_denoms_in_v4_and_v5() {
        let prev = sdk::version::app_version();
        for (version, filters) in [(4, true), (5, false), (9, false), (10, false)] {
            sdk::version::set_app_version(version);
            let mut ms = MultiStore::new();
            let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
            ctx.consensus_params.version = Some(VersionParams { app: version });
            let m = TokenFilter(Accept);
            let native = m
                .on_recv_packet(&mut ctx, &packet("transfer/channel-2/utia"), &[])
                .unwrap()
                .unwrap();
            assert!(native.success, "v{version}");
            let foreign = m
                .on_recv_packet(&mut ctx, &packet("uatom"), &[])
                .unwrap()
                .unwrap();
            assert_eq!(foreign.success, !filters, "v{version}");
            assert_eq!(m.upgradable(), !filters, "v{version}");
            let events = ctx.events.take();
            if filters {
                assert_eq!(events.len(), 1);
                let attrs: Vec<(&str, &str)> = events[0]
                    .attributes
                    .iter()
                    .map(|a| (a.key.as_str(), a.value.as_str()))
                    .collect();
                assert_eq!(events[0].r#type, "fungible_token_packet");
                assert!(attrs.contains(&("module", "tokenfilter")));
                assert!(attrs.contains(&(
                    "error",
                    "only native denom transfers accepted, got uatom: invalid type"
                )));
                assert_eq!(
                    String::from_utf8(foreign.bytes).unwrap(),
                    "{\"error\":\"ABCI code: 29: error handling packet: see events for details\"}"
                );
            } else {
                assert!(events.is_empty(), "v{version}");
            }
        }
        sdk::version::set_app_version(prev);
    }
}
