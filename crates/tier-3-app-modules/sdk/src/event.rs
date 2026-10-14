//! ABCI events (`sdk.EventManager`).

use celestia_core_proto::abci::{Event, EventAttribute};

/// Builds an event with `index = true` attributes, as `sdk.NewEvent` +
/// `sdk.NewAttribute` produce (the SDK marks all attributes indexed by
/// default in `ToABCIEvents`).
pub fn event<K: Into<String>, V: Into<String>>(
    ty: impl Into<String>,
    attrs: impl IntoIterator<Item = (K, V)>,
) -> Event {
    Event {
        r#type: ty.into(),
        attributes: attrs
            .into_iter()
            .map(|(k, v)| EventAttribute {
                key: k.into(),
                value: v.into(),
                index: true,
            })
            .collect(),
    }
}

/// Collects events emitted during execution.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventManager {
    events: Vec<Event>,
}

impl EventManager {
    /// Emits one event.
    pub fn emit(&mut self, e: Event) {
        self.events.push(e);
    }

    /// Emits several events.
    pub fn emit_all(&mut self, es: impl IntoIterator<Item = Event>) {
        self.events.extend(es);
    }

    /// Number of events so far (a checkpoint for [`Self::truncate`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when no events were emitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Drops events after a checkpoint (used when a branch is discarded).
    pub fn truncate(&mut self, len: usize) {
        self.events.truncate(len);
    }

    /// Takes all events.
    pub fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Borrows the events.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
