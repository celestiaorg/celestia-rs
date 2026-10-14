//! `cosmos.base.reflection.v1beta1.ReflectionService` over the interface
//! registry of celestia-appd ([`crate::reflection_table`]).

use celestia_proto::cosmos::base::reflection::v1beta1 as pb;
use sdk::Context;

use crate::Router;
use crate::error::{QResult, invalid_arg};
use crate::reflection_table::INTERFACES;

pub(crate) fn register(r: &mut Router) {
    r.add(
        "/cosmos.base.reflection.v1beta1.ReflectionService/ListAllInterfaces",
        list_all_interfaces,
    );
    r.add(
        "/cosmos.base.reflection.v1beta1.ReflectionService/ListImplementations",
        list_implementations,
    );
}

fn list_all_interfaces(
    _: &mut Context<'_>,
    _: pb::ListAllInterfacesRequest,
) -> QResult<pb::ListAllInterfacesResponse> {
    Ok(pb::ListAllInterfacesResponse {
        interface_names: INTERFACES.iter().map(|(i, _)| (*i).to_owned()).collect(),
    })
}

fn list_implementations(
    _: &mut Context<'_>,
    req: pb::ListImplementationsRequest,
) -> QResult<pb::ListImplementationsResponse> {
    if req.interface_name.is_empty() {
        return Err(invalid_arg("invalid interface name"));
    }
    let impls = INTERFACES
        .iter()
        .find(|(i, _)| *i == req.interface_name)
        .map(|(_, impls)| impls.iter().map(|s| (*s).to_owned()).collect())
        .unwrap_or_default();
    Ok(pb::ListImplementationsResponse {
        implementation_message_names: impls,
    })
}
