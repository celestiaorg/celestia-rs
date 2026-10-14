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

fn read_varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(u8::try_from(v & 0x7f).unwrap_or(0) | 0x80);
        v >>= 7;
    }
    out.push(u8::try_from(v).unwrap_or(0));
}

/// Go's protobuf does not validate UTF-8, so recorded tx logs can hold raw
/// bytes (e.g. authz's "failed to delete grant with key <raw key>"). Makes
/// the `log`/`info` strings of FinalizeBlock tx results and CheckTx valid
/// UTF-8 (lossily) so the response decodes; consensus fields are untouched.
#[must_use]
pub fn sanitize_logs(bytes: &[u8]) -> Vec<u8> {
    // Field paths: Response.check_tx(9).{log 3, info 4};
    // Response.finalize_block(21).tx_results(2).{log 3, info 4};
    // Response.exception(1).error(1).
    fn fields(b: &[u8]) -> Option<Vec<(u64, u8, Vec<u8>)>> {
        let mut i = 0;
        let mut out = Vec::new();
        while i < b.len() {
            let key = read_varint(b, &mut i)?;
            let (num, wt) = (key >> 3, u8::try_from(key & 7).ok()?);
            let start = i;
            match wt {
                0 => {
                    read_varint(b, &mut i)?;
                }
                1 => i += 8,
                2 => {
                    let n = usize::try_from(read_varint(b, &mut i)?).ok()?;
                    let s = i;
                    i = s.checked_add(n)?;
                    out.push((num, wt, b.get(s..i)?.to_vec()));
                    continue;
                }
                5 => i += 4,
                _ => return None,
            }
            out.push((num, wt, b.get(start..i)?.to_vec()));
        }
        Some(out)
    }
    fn encode(fs: &[(u64, u8, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (num, wt, v) in fs {
            write_varint(&mut out, (num << 3) | u64::from(*wt));
            if *wt == 2 {
                write_varint(&mut out, v.len() as u64);
            }
            out.extend_from_slice(v);
        }
        out
    }
    fn lossy(v: &[u8]) -> Vec<u8> {
        String::from_utf8_lossy(v).into_owned().into_bytes()
    }
    fn strings(b: &[u8], nums: &[u64]) -> Option<Vec<u8>> {
        let mut fs = fields(b)?;
        for f in &mut fs {
            if f.1 == 2 && nums.contains(&f.0) {
                f.2 = lossy(&f.2);
            }
        }
        Some(encode(&fs))
    }
    let run = || -> Option<Vec<u8>> {
        let mut top = fields(bytes)?;
        for f in &mut top {
            match (f.0, f.1) {
                (1 | 9, 2) => f.2 = strings(&f.2, if f.0 == 1 { &[1] } else { &[3, 4] })?,
                (21, 2) => {
                    let mut fb = fields(&f.2)?;
                    for r in &mut fb {
                        if r.0 == 2 && r.1 == 2 {
                            r.2 = strings(&r.2, &[3, 4])?;
                        }
                    }
                    f.2 = encode(&fb);
                }
                _ => {}
            }
        }
        Some(encode(&top))
    };
    run().unwrap_or_else(|| bytes.to_vec())
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
                let resp = Response::decode(r.bytes.as_slice())
                    .or_else(|_| Response::decode(sanitize_logs(&r.bytes).as_slice()))
                    .map_err(|e| bad(e.to_string()))?;
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
