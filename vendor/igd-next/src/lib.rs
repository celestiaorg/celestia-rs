#![deny(missing_docs)]

//! This library allows you to communicate with an IGD enabled device.
//! Use the Tokio gateway search to obtain an asynchronous gateway.
//! You can then communicate with the device via this object.

// data structures
pub use self::common::parsing::PortMappingEntry;
pub use self::common::SearchOptions;
pub use self::errors::{
    AddAnyPortError, AddPortError, GetExternalIpError, GetGenericPortMappingEntryError, RemovePortError, RequestError,
    SearchError,
};
pub use self::errors::{Error, Result};

#[cfg(feature = "aio_tokio")]
pub mod aio;
mod common;
mod errors;



use std::fmt;

/// Represents the protocols available for port mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortMappingProtocol {
    /// TCP protocol
    TCP,
    /// UDP protocol
    UDP,
}

impl fmt::Display for PortMappingProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match *self {
                PortMappingProtocol::TCP => "TCP",
                PortMappingProtocol::UDP => "UDP",
            }
        )
    }
}
