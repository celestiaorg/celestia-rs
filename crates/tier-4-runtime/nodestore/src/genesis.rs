//! `types.GenesisDoc` (`types/genesis.go`) and RFC 3339 time parsing.
//!
//! Accepts both a CometBFT genesis document (`consensus_params`,
//! `validators` at the top level) and the Cosmos SDK `AppGenesis` written by
//! `celestia-appd init` / `collect-gentxs` (`consensus.params`,
//! `consensus.validators`), so a node home produced by the Go tooling can be
//! started directly — the same conversion `conformance/devnet/devnet.sh`
//! performs in Python.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ctypes::params::{AbciParams, BlockParams, EvidenceParams, ValidatorParams, VersionParams};
use ctypes::{ConsensusParams, PubKey, Time};
use serde_json::Value;

use crate::{Error, Result};

/// `GenesisValidator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisValidator {
    /// Address.
    pub address: Vec<u8>,
    /// Public key.
    pub pub_key: PubKey,
    /// Voting power.
    pub power: i64,
    /// Name.
    pub name: String,
}

/// `GenesisDoc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisDoc {
    /// Genesis time.
    pub genesis_time: Time,
    /// Chain id.
    pub chain_id: String,
    /// Initial height.
    pub initial_height: i64,
    /// Consensus params.
    pub consensus_params: ConsensusParams,
    /// Validators (may be empty: InitChain provides them).
    pub validators: Vec<GenesisValidator>,
    /// Initial app hash.
    pub app_hash: Vec<u8>,
    /// Raw `app_state` JSON passed to InitChain.
    pub app_state: Vec<u8>,
}

fn int_field(v: &Value, what: &str) -> Result<i64> {
    match v {
        Value::Null => Ok(0),
        Value::String(s) => s
            .parse()
            .map_err(|e| Error::new(format!("genesis: invalid {what} {s:?}: {e}"))),
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| Error::new(format!("genesis: invalid {what} {n}"))),
        other => Err(Error::new(format!("genesis: invalid {what} {other}"))),
    }
}

fn params_from_json(p: &Value) -> Result<ConsensusParams> {
    let b = &p["block"];
    let e = &p["evidence"];
    let mut out = ConsensusParams {
        block: BlockParams {
            max_bytes: int_field(&b["max_bytes"], "block.max_bytes")?,
            max_gas: int_field(&b["max_gas"], "block.max_gas")?,
        },
        evidence: EvidenceParams {
            max_age_num_blocks: int_field(&e["max_age_num_blocks"], "evidence.max_age_num_blocks")?,
            max_age_duration: int_field(&e["max_age_duration"], "evidence.max_age_duration")?,
            max_bytes: int_field(&e["max_bytes"], "evidence.max_bytes")?,
        },
        validator: ValidatorParams {
            pub_key_types: p["validator"]["pub_key_types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        },
        version: VersionParams {
            app: u64::try_from(int_field(&p["version"]["app"], "version.app")?).unwrap_or(0),
        },
        abci: AbciParams {
            vote_extensions_enable_height: int_field(
                &p["abci"]["vote_extensions_enable_height"],
                "abci.vote_extensions_enable_height",
            )?,
        },
    };
    if p["block"].is_null() {
        out.block = ConsensusParams::default_params().block;
    }
    if p["evidence"].is_null() {
        out.evidence = ConsensusParams::default_params().evidence;
    }
    if p["validator"].is_null() {
        out.validator = ConsensusParams::default_params().validator;
    }
    Ok(out)
}

/// Amino JSON public key (`{"type": "tendermint/PubKeyEd25519", "value": b64}`).
pub fn pub_key_from_json(v: &Value) -> Result<PubKey> {
    let raw = B64
        .decode(v["value"].as_str().unwrap_or_default())
        .map_err(|e| Error::new(format!("genesis: pub_key: {e}")))?;
    match v["type"].as_str() {
        Some("tendermint/PubKeyEd25519") => <[u8; 32]>::try_from(raw.as_slice())
            .map(PubKey::Ed25519)
            .map_err(|_| Error::new("genesis: invalid ed25519 key length")),
        Some("tendermint/PubKeySecp256k1") => <[u8; 33]>::try_from(raw.as_slice())
            .map(PubKey::Secp256k1)
            .map_err(|_| Error::new("genesis: invalid secp256k1 key length")),
        t => Err(Error::new(format!("genesis: unsupported key type {t:?}"))),
    }
}

impl GenesisDoc {
    /// `GenesisDocFromJSON` + `ValidateAndComplete`, accepting the SDK
    /// `AppGenesis` layout too. `now` fills a missing genesis time.
    pub fn from_json(bz: &[u8], now: Time) -> Result<Self> {
        let g: Value =
            serde_json::from_slice(bz).map_err(|e| Error::new(format!("genesis: {e}")))?;
        let (params, vals) = if g.get("consensus").is_some_and(|c| !c.is_null()) {
            (&g["consensus"]["params"], &g["consensus"]["validators"])
        } else {
            (&g["consensus_params"], &g["validators"])
        };
        let consensus_params = if params.is_null() {
            ConsensusParams::default_params()
        } else {
            params_from_json(params)?
        };
        let mut validators = Vec::new();
        for v in vals.as_array().into_iter().flatten() {
            let pub_key = pub_key_from_json(&v["pub_key"])?;
            let address = match v["address"].as_str() {
                Some(a) if !a.is_empty() => {
                    hex::decode(a).map_err(|e| Error::new(format!("genesis: address: {e}")))?
                }
                _ => Vec::new(),
            };
            validators.push(GenesisValidator {
                address,
                pub_key,
                power: int_field(&v["power"], "validator power")?,
                name: v["name"].as_str().unwrap_or_default().to_owned(),
            });
        }
        let genesis_time = match g["genesis_time"].as_str() {
            Some(s) if !s.is_empty() => parse_rfc3339(s)?,
            _ => Time::ZERO,
        };
        let app_hash = match g["app_hash"].as_str() {
            Some(s) if !s.is_empty() => {
                hex::decode(s).map_err(|e| Error::new(format!("genesis: app_hash: {e}")))?
            }
            _ => Vec::new(),
        };
        let app_state = if g["app_state"].is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(&g["app_state"]).map_err(|e| Error::new(e.to_string()))?
        };
        let mut doc = Self {
            genesis_time,
            chain_id: g["chain_id"].as_str().unwrap_or_default().to_owned(),
            initial_height: int_field(&g["initial_height"], "initial_height")?,
            consensus_params,
            validators,
            app_hash,
            app_state,
        };
        doc.validate_and_complete(now)?;
        Ok(doc)
    }

    /// `ValidateAndComplete`.
    pub fn validate_and_complete(&mut self, now: Time) -> Result<()> {
        if self.chain_id.is_empty() {
            return Err(Error::new("genesis doc must include non-empty chain_id"));
        }
        if self.chain_id.len() > ctypes::header::MAX_CHAIN_ID_LEN {
            return Err(Error::new(format!(
                "chain_id in genesis doc is too long (max: {})",
                ctypes::header::MAX_CHAIN_ID_LEN
            )));
        }
        if self.initial_height < 0 {
            return Err(Error::new(format!(
                "initial_height cannot be negative (got {})",
                self.initial_height
            )));
        }
        if self.initial_height == 0 {
            self.initial_height = 1;
        }
        self.consensus_params
            .validate_basic()
            .map_err(|e| Error::new(e.to_string()))?;
        for (i, v) in self.validators.iter_mut().enumerate() {
            if v.power == 0 {
                return Err(Error::new(format!(
                    "the genesis file cannot contain validators with no voting power: {}",
                    v.name
                )));
            }
            if !v.address.is_empty() && v.pub_key.address() != v.address {
                return Err(Error::new(format!(
                    "incorrect address for validator #{i} in the genesis file, should be {}",
                    hex::encode_upper(v.pub_key.address())
                )));
            }
            if v.address.is_empty() {
                v.address = v.pub_key.address();
            }
        }
        if self.genesis_time.is_zero() {
            self.genesis_time = now;
        }
        Ok(())
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses an RFC 3339 timestamp (`2006-01-02T15:04:05.999999999Z07:00`).
pub fn parse_rfc3339(s: &str) -> Result<Time> {
    let err = || Error::new(format!("invalid RFC 3339 time {s:?}"));
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b't') {
        return Err(err());
    }
    let num = |r: core::ops::Range<usize>| -> Result<i64> {
        s.get(r).and_then(|x| x.parse().ok()).ok_or_else(err)
    };
    let (y, mo, d, h, mi, se) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let mut i = 19;
    let mut nanos: i64 = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[start..i];
        if frac.is_empty() || frac.len() > 9 {
            return Err(err());
        }
        nanos = frac.parse::<i64>().map_err(|_| err())?
            * 10_i64.pow(9 - u32::try_from(frac.len()).unwrap_or(9));
    }
    let offset = match b.get(i) {
        Some(b'Z' | b'z') if i + 1 == b.len() => 0,
        Some(c @ (b'+' | b'-')) if i + 6 == b.len() => {
            let oh = num(i + 1..i + 3)?;
            let om = num(i + 4..i + 6)?;
            let o = oh * 3600 + om * 60;
            if *c == b'+' { o } else { -o }
        }
        _ => return Err(err()),
    };
    let seconds = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset;
    Ok(Time::from_unix(
        seconds,
        i32::try_from(nanos).map_err(|_| err())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        for s in [
            "2026-10-14T10:39:47.472383303Z",
            "1970-01-01T00:00:00Z",
            "2023-11-14T22:13:20.12Z",
            "0001-01-01T00:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(s).unwrap().canonical_string(), s);
        }
        assert_eq!(
            parse_rfc3339("2023-11-14T23:13:20.12+01:00").unwrap(),
            parse_rfc3339("2023-11-14T22:13:20.12Z").unwrap()
        );
        assert!(parse_rfc3339("2023-11-14 22:13:20Z").is_err());
    }

    #[test]
    fn sdk_app_genesis() {
        let g = br#"{"app_name":"<appd>","genesis_time":"2026-10-14T10:39:47.472383303Z",
          "chain_id":"rs-devnet-1","initial_height":1,"app_hash":null,
          "consensus":{"params":{"block":{"max_bytes":"33554432","max_gas":"-1"},
          "evidence":{"max_age_num_blocks":"100000","max_age_duration":"172800000000000","max_bytes":"1048576"},
          "validator":{"pub_key_types":["ed25519"]},"version":{"app":"10"},
          "abci":{"vote_extensions_enable_height":"0"}}},"app_state":{"auth":{}}}"#;
        let d = GenesisDoc::from_json(g, Time::ZERO).unwrap();
        assert_eq!(d.chain_id, "rs-devnet-1");
        assert_eq!(d.initial_height, 1);
        assert_eq!(d.consensus_params.block.max_bytes, 33_554_432);
        assert_eq!(d.consensus_params.version.app, 10);
        assert_eq!(
            d.consensus_params.evidence.max_age_duration,
            172_800_000_000_000
        );
        assert!(d.validators.is_empty());
        assert_eq!(d.app_state, br#"{"auth":{}}"#);
    }
}
