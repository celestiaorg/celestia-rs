//! Observe the multistream-select acknowledgement at the Yamux stream boundary.
use std::sync::Arc;

const HEADER: &[u8] = b"/multistream/1.0.0\n";

#[derive(Default)]
struct Frame {
    length: usize,
    continuation: bool,
    ready: bool,
    body: Vec<u8>,
}

impl Frame {
    // multistream-select bounds its length prefix to two bytes (16,383 bytes).
    fn take(&mut self, input: &mut &[u8]) -> Result<Option<Vec<u8>>, ()> {
        while !self.ready {
            let Some((&b, rest)) = input.split_first() else {
                return Ok(None);
            };
            *input = rest;
            if self.continuation && b & 0x80 != 0 {
                return Err(());
            }
            self.length |= usize::from(b & 0x7f) << if self.continuation { 7 } else { 0 };
            self.continuation = b & 0x80 != 0;
            self.ready = !self.continuation;
        }
        let n = input.len().min(self.length - self.body.len());
        self.body.extend_from_slice(&input[..n]);
        *input = &input[n..];
        if self.body.len() < self.length {
            return Ok(None);
        }
        Ok(Some(std::mem::take(self).body))
    }
}

#[derive(Default)]
struct Handshake {
    frames: [Frame; 2],
    headers: [bool; 2],
    offer: Option<Vec<u8>>,
    early: usize,
}

pub(super) struct Negotiation {
    initiator_sends: bool,
    pending: Option<Box<Handshake>>,
    pub(super) protocol: Option<Arc<str>>,
}

impl Negotiation {
    pub(super) fn new(outbound: bool) -> Self {
        Self {
            initiator_sends: outbound,
            pending: Some(Box::default()),
            protocol: None,
        }
    }

    pub(super) fn reserved(&self) -> usize {
        self.pending.as_ref().map_or(0, |h| {
            size_of::<Handshake>()
                + h.frames.iter().map(|f| f.body.capacity()).sum::<usize>()
                + h.offer.as_ref().map_or(0, Vec::capacity)
        })
    }

    pub(super) fn unconfirmed(&self) -> [usize; 2] {
        let mut bytes = [0; 2];
        bytes[usize::from(self.initiator_sends)] = self.pending.as_ref().map_or(0, |h| h.early);
        bytes
    }

    // Return application bytes in receive/send order. A lazy dialer may write
    // application data with its offer. Attribute it only after the peer accepts.
    pub(super) fn observe(&mut self, sent: bool, mut input: &[u8]) -> [usize; 2] {
        let dir = usize::from(sent);
        let mut payload = [0; 2];
        if self.protocol.is_some() {
            payload[dir] = input.len();
            return payload;
        }
        let Some(h) = &mut self.pending else {
            return payload;
        };
        let proposer = sent == self.initiator_sends;
        while !input.is_empty() {
            if proposer && h.offer.is_some() {
                h.early = h.early.saturating_add(input.len());
                break;
            }
            let frame = match h.frames[dir].take(&mut input) {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(()) => {
                    self.pending = None;
                    break;
                }
            };
            if !h.headers[dir] {
                if frame != HEADER {
                    self.pending = None;
                    break;
                }
                h.headers[dir] = true;
            } else if proposer && frame.starts_with(b"/") && frame.ends_with(b"\n") {
                h.offer = Some(frame);
            } else if !proposer && h.offer.as_ref() == Some(&frame) {
                if let Ok(protocol) = std::str::from_utf8(&frame[..frame.len() - 1]) {
                    self.protocol = Some(Arc::from(protocol));
                    payload[usize::from(self.initiator_sends)] = h.early;
                    payload[dir] = input.len();
                }
                self.pending = None;
                break;
            } else if !proposer && frame == b"na\n" {
                h.offer = None;
                h.early = 0;
            }
        }
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(s: &[u8]) -> Vec<u8> {
        let n = u8::try_from(s.len()).unwrap();
        [vec![n], s.to_vec()].concat()
    }

    #[test]
    fn meters_only_accepted_application_bytes_in_both_directions() {
        for outbound in [true, false] {
            let mut negotiation = Negotiation::new(outbound);
            for b in frame(HEADER) {
                assert_eq!(negotiation.observe(outbound, &[b]), [0; 2]);
            }
            negotiation.observe(!outbound, &frame(HEADER));
            negotiation.observe(outbound, &frame(b"/absent\n"));
            negotiation.observe(!outbound, &frame(b"na\n"));
            let mut offer = frame(b"/ipfs/ping/1.0.0\n");
            offer.extend_from_slice(&[0; 32]);
            assert_eq!(negotiation.observe(outbound, &offer), [0; 2]);
            let mut accepted = frame(b"/ipfs/ping/1.0.0\n");
            accepted.extend_from_slice(&[0; 9]);
            let actual = negotiation.observe(!outbound, &accepted);
            assert_eq!(actual[usize::from(outbound)], 32);
            assert_eq!(actual[usize::from(!outbound)], 9);
            assert_eq!(negotiation.protocol.as_deref(), Some("/ipfs/ping/1.0.0"));
        }
    }
}
