//! transfer 2 to 5 (ibc-go v8.7.0 `apps/transfer/keeper/migrations.go`).

use celestia_proto::ibc::applications::transfer::v1::Params;
use sdk::{Context, Result};

/// The transfer port (`types.PortID`).
const PORT: &str = "transfer";

/// transfer 2 to 3 (`MigrateTotalEscrowForDenom`): the balances of the
/// escrow account of every channel whose port starts with the bound port,
/// summed per denom. 3 to 4 (`MigrateParams`): the params out of the
/// subspace `transfer`. 4 to 5 (`MigrateDenomMetadata`): bank metadata for
/// every denom trace that has none.
pub fn migrate_transfer(ctx: &mut Context<'_>) -> Result<()> {
    let port = x_transfer::get_port(ctx)?;
    // `GetAllChannelsWithPortPrefix`: every port when the prefix is blank.
    let prefix = if port.trim().is_empty() {
        "channelEnds/ports/".to_owned()
    } else {
        format!("channelEnds/ports/{port}")
    };
    let mut channels = Vec::new();
    {
        let mut it = ctx.prefix_iter(x_ibc::STORE_KEY, prefix.as_bytes())?;
        while let Some((k, _)) = it.next(ctx)? {
            let path = String::from_utf8_lossy(&k).into_owned();
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() == 5 && parts[1] == "ports" && parts[3] == "channels" {
                channels.push((parts[2].to_owned(), parts[4].to_owned()));
            }
        }
    }
    let mut total = sdk::coins::Coins::new();
    for (_, channel) in &channels {
        // `GetEscrowAddress(portID, channelID)` with the bound port.
        let escrow = x_transfer::escrow_address(&port, channel);
        total = total.add(&x_bank::get_all_balances(ctx, &escrow)?)?;
    }
    for coin in &total {
        x_transfer::set_total_escrow_for_denom(
            ctx,
            &celestia_proto::cosmos::base::v1beta1::Coin {
                denom: coin.denom.clone(),
                amount: coin.amount.to_string(),
            },
        )?;
    }

    let params = Params {
        send_enabled: x_params::v046::get_bool(ctx, PORT, "SendEnabled")?,
        receive_enabled: x_params::v046::get_bool(ctx, PORT, "ReceiveEnabled")?,
    };
    x_transfer::set_params(ctx, &params)?;

    for trace in x_transfer::get_all_denom_traces(ctx)? {
        let denom = x_transfer::ibc_denom(&trace);
        // bank `HasDenomMetaData` (`DenomMetadataPrefix || denom`).
        let metadata_key = sdk::codec::key(&[&[0x01], denom.as_bytes()]);
        if !ctx.has(x_bank::STORE_KEY, &metadata_key)? {
            x_bank::set_denom_metadata(ctx, &x_transfer::denom_metadata(&trace))?;
        }
    }
    Ok(())
}
