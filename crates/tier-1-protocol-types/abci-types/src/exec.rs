//! Transaction execution result types.

use crate::{Event, Result, proto};

/// Result of executing one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTxResult {
    /// Application response code.
    pub code: u32,
    /// Opaque response data bytes.
    pub data: Vec<u8>,
    /// Nondeterministic log string.
    pub log: String,
    /// Nondeterministic info string.
    pub info: String,
    /// Requested gas.
    pub gas_wanted: i64,
    /// Consumed gas.
    pub gas_used: i64,
    /// Ordered transaction events.
    pub events: Vec<Event>,
    /// Application codespace.
    pub codespace: String,
}

impl ExecTxResult {
    /// Perform stateless transaction result validation.
    pub fn validate_basic(&self) -> Result<()> {
        for event in &self.events {
            event.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into a domain transaction result.
    pub fn from_proto(proto: proto::abci::ExecTxResult) -> Result<Self> {
        let result = Self {
            code: proto.code,
            data: proto.data.to_vec(),
            log: proto.log,
            info: proto.info,
            gas_wanted: proto.gas_wanted,
            gas_used: proto.gas_used,
            events: proto
                .events
                .into_iter()
                .map(Event::from_proto)
                .collect::<Result<Vec<_>>>()?,
            codespace: proto.codespace,
        };
        result.validate_basic()?;
        Ok(result)
    }

    /// Convert this transaction result to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::ExecTxResult {
        proto::abci::ExecTxResult {
            code: self.code,
            data: self.data.clone().into(),
            log: self.log.clone(),
            info: self.info.clone(),
            gas_wanted: self.gas_wanted,
            gas_used: self.gas_used,
            events: self.events.iter().map(Event::to_proto).collect(),
            codespace: self.codespace.clone(),
        }
    }
}
