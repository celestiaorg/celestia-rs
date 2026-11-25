//! `MsgRegisterEVMAddress` (`types/msgs.go`, `keeper/msg_server.go`). The
//! message is not routed yet: the v1 message router and ante chain come
//! with the v1 transaction execution.

use sdk::{Context, Result};

use crate::errors;
use crate::types::{self, MsgRegisterEvmAddress, MsgRegisterEvmAddressResponse};

/// `ValidateBasic`: a bech32 validator address and a hex EVM address.
pub fn validate_basic(msg: &MsgRegisterEvmAddress) -> Result<()> {
    x_staking::addr::val_from_string(&msg.validator_address)?;
    if !types::is_hex_address(&msg.evm_address) {
        return Err(errors::evm_address_not_hex());
    }
    Ok(())
}

/// `GetSigners`: the validator's operator address as an account.
pub fn signers(msg: &MsgRegisterEvmAddress) -> Result<Vec<Vec<u8>>> {
    Ok(vec![x_staking::addr::val_from_string(
        &msg.validator_address,
    )?])
}

/// `msgServer.RegisterEVMAddress`: sets the validator's EVM address if no
/// validator has it yet.
pub fn register_evm_address(
    ctx: &mut Context<'_>,
    msg: &MsgRegisterEvmAddress,
) -> Result<MsgRegisterEvmAddressResponse> {
    let val = x_staking::addr::val_from_string(&msg.validator_address)?;
    let evm = types::hex_to_address(&msg.evm_address);
    if x_staking::store::get_validator(ctx, &val)?.is_none() {
        return Err(x_staking::errors::no_validator_found());
    }
    if !crate::is_evm_address_unique(ctx, &evm)? {
        return Err(errors::evm_address_already_exists(&format!(
            "address {}",
            msg.evm_address
        )));
    }
    crate::set_evm_address(ctx, &val, &evm)?;
    Ok(MsgRegisterEvmAddressResponse {})
}
