//! Recording format for ABCI traffic captured by `abci-recorder`.
//!
//! A recording is a flat sequence of records:
//! `conn: u32 LE | dir: u8 (0 = request, 1 = response) | len: u32 LE | bytes`.
//! Records of one connection appear in wire order, which (ABCI being
//! strictly ordered per connection) pairs the n-th request with the n-th
//! response.

use std::io;

use celestia_core_proto::abci::{Request, Response};
use prost::Message;

/// Direction of a recorded frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// celestia-core → app.
    Request,
    /// app → celestia-core.
    Response,
}

/// One recorded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Connection index in accept order.
    pub conn: u32,
    /// Frame direction.
    pub dir: Direction,
    /// Raw protobuf bytes (without length prefix).
    pub bytes: Vec<u8>,
}

impl Record {
    /// Serializes this record.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.bytes.len());
        out.extend_from_slice(&self.conn.to_le_bytes());
        out.push(match self.dir {
            Direction::Request => 0,
            Direction::Response => 1,
        });
        let len = u32::try_from(self.bytes.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&self.bytes);
        out
    }
}

/// Parses a whole recording.
pub fn parse(mut data: &[u8]) -> io::Result<Vec<Record>> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut out = Vec::new();
    while !data.is_empty() {
        if data.len() < 9 {
            return Err(bad("truncated record header"));
        }
        let conn = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let dir = match data[4] {
            0 => Direction::Request,
            1 => Direction::Response,
            _ => return Err(bad("bad direction byte")),
        };
        let len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;
        data = &data[9..];
        if data.len() < len {
            return Err(bad("truncated record body"));
        }
        out.push(Record {
            conn,
            dir,
            bytes: data[..len].to_vec(),
        });
        data = &data[len..];
    }
    Ok(out)
}

/// A decoded request/response exchange.
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    /// Connection index.
    pub conn: u32,
    /// The request.
    pub request: Request,
    /// The app's response.
    pub response: Response,
}

/// Pairs requests with responses per connection, preserving the global
/// order of requests.
pub fn exchanges(records: &[Record]) -> io::Result<Vec<Exchange>> {
    use std::collections::{BTreeMap, VecDeque};
    let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
    let mut pending: BTreeMap<u32, VecDeque<usize>> = BTreeMap::new();
    let mut slots: Vec<(u32, Request, Option<Response>)> = Vec::new();
    for r in records {
        match r.dir {
            Direction::Request => {
                let req = Request::decode(r.bytes.as_slice()).map_err(|e| bad(e.to_string()))?;
                pending.entry(r.conn).or_default().push_back(slots.len());
                slots.push((r.conn, req, None));
            }
            Direction::Response => {
                let resp = Response::decode(r.bytes.as_slice()).map_err(|e| bad(e.to_string()))?;
                let idx = pending
                    .get_mut(&r.conn)
                    .and_then(VecDeque::pop_front)
                    .ok_or_else(|| bad(format!("response without request on conn {}", r.conn)))?;
                slots[idx].2 = Some(resp);
            }
        }
    }
    Ok(slots
        .into_iter()
        .filter_map(|(conn, request, response)| {
            response.map(|response| Exchange {
                conn,
                request,
                response,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_core_proto::abci::{RequestEcho, ResponseEcho, request, response};

    #[test]
    fn roundtrip_and_pair() -> io::Result<()> {
        let req = Request {
            value: Some(request::Value::Echo(RequestEcho {
                message: "hi".into(),
            })),
        };
        let resp = Response {
            value: Some(response::Value::Echo(ResponseEcho {
                message: "hi".into(),
            })),
        };
        let recs = vec![
            Record {
                conn: 0,
                dir: Direction::Request,
                bytes: req.encode_to_vec(),
            },
            Record {
                conn: 1,
                dir: Direction::Request,
                bytes: req.encode_to_vec(),
            },
            Record {
                conn: 1,
                dir: Direction::Response,
                bytes: resp.encode_to_vec(),
            },
            Record {
                conn: 0,
                dir: Direction::Response,
                bytes: resp.encode_to_vec(),
            },
        ];
        let wire: Vec<u8> = recs.iter().flat_map(Record::encode).collect();
        assert_eq!(parse(&wire)?, recs);
        let ex = exchanges(&recs)?;
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0].conn, 0);
        assert_eq!(ex[1].conn, 1);
        assert_eq!(ex[0].response, resp);
        Ok(())
    }

    #[test]
    fn rejects_truncation() {
        assert!(parse(&[0, 0, 0, 0, 0, 5, 0, 0, 0, 1]).is_err());
        assert!(parse(&[0, 0]).is_err());
        assert!(parse(&[0, 0, 0, 0, 7, 0, 0, 0, 0]).is_err());
    }
}
