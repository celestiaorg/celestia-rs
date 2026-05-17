//! ABCI event types.

use crate::{Error, Result, proto};

/// A single key-value pair attached to an ABCI event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAttribute {
    /// Attribute key.
    pub key: String,
    /// Attribute value.
    pub value: String,
    /// Whether CometBFT should index this attribute.
    pub index: bool,
}

impl EventAttribute {
    /// Perform stateless attribute validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.key.is_empty() {
            return Err(Error::InvalidEvent);
        }
        Ok(())
    }

    /// Convert protobuf into a domain event attribute.
    pub fn from_proto(proto: proto::abci::EventAttribute) -> Result<Self> {
        let attribute = Self {
            key: proto.key,
            value: proto.value,
            index: proto.index,
        };
        attribute.validate_basic()?;
        Ok(attribute)
    }

    /// Convert this event attribute to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::EventAttribute {
        proto::abci::EventAttribute {
            key: self.key.clone(),
            value: self.value.clone(),
            index: self.index,
        }
    }
}

/// An ABCI event emitted by the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Event type.
    pub ty: String,
    /// Ordered event attributes.
    pub attributes: Vec<EventAttribute>,
}

impl Event {
    /// Perform stateless event validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.ty.is_empty() {
            return Err(Error::InvalidEvent);
        }
        for attribute in &self.attributes {
            attribute.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into a domain event.
    pub fn from_proto(proto: proto::abci::Event) -> Result<Self> {
        let event = Self {
            ty: proto.r#type,
            attributes: proto
                .attributes
                .into_iter()
                .map(EventAttribute::from_proto)
                .collect::<Result<Vec<_>>>()?,
        };
        event.validate_basic()?;
        Ok(event)
    }

    /// Convert this event to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Event {
        proto::abci::Event {
            r#type: self.ty.clone(),
            attributes: self
                .attributes
                .iter()
                .map(EventAttribute::to_proto)
                .collect(),
        }
    }
}
