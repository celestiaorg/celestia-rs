//! Port of celestia-core v0.42.3 `state/txindex/kv` and
//! `state/indexer/block/kv`: the kv tx and block indexers (events with the
//! `index` flag), their search semantics (condition intersection keyed by
//! hash/height + event sequence, height de-duplication, numeric ranges) and
//! paging. Both live in one [`KvBackend`] like Go's `tx_index.db`, the block
//! index under the `block_events` prefix.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering as AtOrd};

use ctypes::proto::abci;
use kvdb::{KvBackend, WriteBatch};
use prost::Message as _;

use crate::query::{Condition, Dec, Query, Token};

/// `types.TxHeightKey`.
pub const TX_HEIGHT_KEY: &str = "tx.height";
/// `types.TxHashKey`.
pub const TX_HASH_KEY: &str = "tx.hash";
/// `types.BlockHeightKey`.
pub const BLOCK_HEIGHT_KEY: &str = "block.height";

const EVENT_SEQ_SEP: &str = "$es$";

fn kv_err(e: kvdb::Error) -> String {
    e.to_string()
}

fn iterate_prefix(db: &dyn KvBackend, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    db.prefix_range(prefix, false)
        .map_err(kv_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(kv_err)
}

// --- query ranges (`state/indexer/query_range.go`) -------------------------

/// A numeric/time range over one tag.
#[derive(Debug, Clone, Default)]
struct QueryRange {
    key: String,
    lower: Option<Bound>,
    upper: Option<Bound>,
    include_lower: bool,
    include_upper: bool,
}

#[derive(Debug, Clone)]
enum Bound {
    Num(Dec),
    /// Times/dates (never match a numeric event value).
    Other,
}

impl QueryRange {
    fn any_bound_is_number(&self) -> bool {
        matches!(
            self.lower.as_ref().or(self.upper.as_ref()),
            Some(Bound::Num(_))
        )
    }
}

fn condition_bound(c: &Condition) -> Option<Bound> {
    let a = c.arg.as_ref()?;
    Some(match a.kind {
        Token::Number => Bound::Num(a.number()?),
        _ => Bound::Other,
    })
}

fn is_range_op(op: Token) -> bool {
    matches!(op, Token::Gt | Token::Geq | Token::Lt | Token::Leq)
}

/// `LookForRangesWithHeight`: ranges in first-appearance order (Go iterates
/// a map; the order only matters for which range initializes the result,
/// which does not change the outcome).
fn look_for_ranges_with_height(
    conds: &[Condition],
    height_key: &str,
) -> (Vec<QueryRange>, Vec<usize>, QueryRange) {
    let mut ranges: Vec<QueryRange> = Vec::new();
    let mut indexes = Vec::new();
    let mut height_range = QueryRange::default();
    for (i, c) in conds.iter().enumerate() {
        if !is_range_op(c.op) {
            continue;
        }
        let is_height = c.tag == BLOCK_HEIGHT_KEY || c.tag == TX_HEIGHT_KEY;
        let pos = if let Some(p) = ranges.iter().position(|r| r.key == c.tag) {
            p
        } else {
            ranges.push(QueryRange {
                key: c.tag.clone(),
                ..QueryRange::default()
            });
            if is_height {
                height_range = QueryRange {
                    key: c.tag.clone(),
                    ..QueryRange::default()
                };
            }
            ranges.len() - 1
        };
        let r = &mut ranges[pos];
        let b = condition_bound(c);
        match c.op {
            Token::Gt => {
                if is_height {
                    height_range.lower.clone_from(&b);
                }
                r.lower = b;
            }
            Token::Geq => {
                r.include_lower = true;
                r.lower.clone_from(&b);
                if is_height {
                    height_range.include_lower = true;
                    height_range.lower = b;
                }
            }
            Token::Lt => {
                r.upper.clone_from(&b);
                if is_height {
                    height_range.upper = b;
                }
            }
            Token::Leq => {
                r.include_upper = true;
                r.upper.clone_from(&b);
                if is_height {
                    height_range.include_upper = true;
                    height_range.upper = b;
                }
            }
            _ => {}
        }
        indexes.push(i);
    }
    let _ = height_key;
    (ranges, indexes, height_range)
}

/// `idxutil.CheckBounds` with a numeric value (Go compares as floats with
/// the bound being a `*big.Float`, so equality excludes non-inclusive
/// bounds).
fn check_bounds(r: &QueryRange, v: &Dec) -> Result<bool, String> {
    for (bound, lower) in [(&r.lower, true), (&r.upper, false)] {
        let Some(b) = bound else { continue };
        let Bound::Num(b) = b else {
            return Err("unable to parse arguments, bad type: time.Time".into());
        };
        let cmp = v.cmp(b);
        if lower {
            if cmp == std::cmp::Ordering::Less
                || (cmp == std::cmp::Ordering::Equal && !r.include_lower)
            {
                return Ok(false);
            }
        } else if cmp == std::cmp::Ordering::Greater
            || (cmp == std::cmp::Ordering::Equal && !r.include_upper)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Default)]
struct HeightInfo {
    height_range: QueryRange,
    height: i64,
    height_eq_idx: Option<usize>,
    only_height_range: bool,
    only_height_eq: bool,
}

/// `dedupHeight` (tx and block flavours differ only in the key and the
/// returned `found` flag).
fn dedup_height(conds: &[Condition], key: &str) -> (Vec<Condition>, HeightInfo, bool) {
    let mut out = Vec::new();
    let mut info = HeightInfo {
        only_height_eq: true,
        only_height_range: true,
        ..HeightInfo::default()
    };
    let mut height_range_exists = false;
    let mut found = false;
    let mut height_cond = Vec::new();
    for c in conds {
        if c.tag == key {
            if c.op == Token::Eq {
                if found || height_range_exists {
                    continue;
                }
                if let Some(h) = c.arg.as_ref().and_then(crate::query::Arg::number) {
                    info.height = h.to_i64();
                    found = true;
                    height_cond.push(c.clone());
                }
            } else {
                info.only_height_eq = false;
                height_range_exists = true;
                out.push(c.clone());
            }
        } else {
            info.only_height_range = false;
            info.only_height_eq = false;
            out.push(c.clone());
        }
    }
    if !height_range_exists && !height_cond.is_empty() {
        info.height_eq_idx = Some(out.len());
        info.only_height_range = false;
        out.extend(height_cond);
    } else {
        info.height_eq_idx = None;
        info.height = 0;
        info.only_height_eq = false;
        found = false;
    }
    (out, info, found)
}

fn check_height_conditions(info: &HeightInfo, key_height: i64) -> Result<bool, String> {
    if info.height_range.key.is_empty() {
        Ok(!(info.height != 0 && key_height != info.height))
    } else {
        check_bounds(&info.height_range, &Dec::from_i64(key_height))
    }
}

/// `indexer.ErrTooManyResults`.
pub const ERR_TOO_MANY_RESULTS: &str = "too many results, please use a more specific query";

fn limit_reached(n: usize, max: usize) -> bool {
    max > 0 && n > max
}

// --- tx indexer ------------------------------------------------------------

#[derive(Debug, Clone)]
struct TxRef {
    hash: Vec<u8>,
    height: i64,
    index: u32,
}

/// `txindex/kv.TxIndex`.
#[derive(Debug)]
pub struct TxIndexer {
    db: Arc<dyn KvBackend>,
    event_seq: AtomicI64,
    /// `max_search_results` (0 = unlimited).
    pub max_search_results: usize,
}

fn tx_hash(tx: &[u8]) -> Vec<u8> {
    ctypes::Tx(tx.to_vec()).hash()
}

fn key_for_event(key: &str, value: &str, r: &abci::TxResult, seq: i64) -> Vec<u8> {
    format!("{key}/{value}/{}/{}{EVENT_SEQ_SEP}{seq}", r.height, r.index).into_bytes()
}

fn key_for_height(r: &abci::TxResult) -> Vec<u8> {
    format!(
        "{TX_HEIGHT_KEY}/{}/{}/{}{EVENT_SEQ_SEP}0",
        r.height, r.height, r.index
    )
    .into_bytes()
}

fn start_key(fields: &[&str]) -> Vec<u8> {
    let mut s = String::new();
    for f in fields {
        s.push_str(f);
        s.push('/');
    }
    s.into_bytes()
}

fn is_tag_key(key: &[u8]) -> bool {
    key.iter().filter(|&&b| b == b'/').count() >= 3
}

fn last_index_of(b: &[u8], c: u8) -> Option<usize> {
    b.iter().rposition(|&x| x == c)
}

fn extract_height_from_key(key: &[u8]) -> Result<i64, String> {
    let end = last_index_of(key, b'/').ok_or("separator not found")?;
    if end == 0 {
        return Err("second last separator not found".into());
    }
    let start = last_index_of(&key[..end - 1], b'/').ok_or("second last separator not found")?;
    std::str::from_utf8(&key[start + 1..end])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "invalid height".to_owned())
}

fn extract_index_from_key(key: &[u8]) -> Result<u32, String> {
    let start = last_index_of(key, b'/').ok_or("separator not found")?;
    let mut idx = &key[start + 1..];
    if let Some(p) = idx
        .windows(EVENT_SEQ_SEP.len())
        .position(|w| w == EVENT_SEQ_SEP.as_bytes())
    {
        idx = &idx[..p];
    }
    std::str::from_utf8(idx)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "invalid index".to_owned())
}

fn extract_value_from_key(key: &[u8]) -> String {
    let idx: Vec<usize> = key
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'/')
        .map(|(i, _)| i)
        .collect();
    if idx.len() < 2 {
        return String::new();
    }
    let v = &key[idx[0] + 1..idx[idx.len() - 2]];
    let v = trim_go_space(v);
    String::from_utf8_lossy(v).into_owned()
}

fn trim_go_space(b: &[u8]) -> &[u8] {
    let s = String::from_utf8_lossy(b);
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    if s.len() == b.len() {
        &b[lead..b.len() - trail]
    } else {
        b
    }
}

fn extract_event_seq_from_key(key: &[u8]) -> String {
    let s = String::from_utf8_lossy(key);
    let last = s.rsplit('/').next().unwrap_or("");
    match last.split_once(EVENT_SEQ_SEP) {
        Some((_, seq)) => seq.to_owned(),
        None => "0".to_owned(),
    }
}

fn tx_ref_less(a: &TxRef, b: &TxRef) -> bool {
    if a.height == b.height {
        a.index < b.index
    } else {
        a.height < b.height
    }
}

impl TxIndexer {
    /// `NewTxIndex`.
    #[must_use]
    pub fn new(db: Arc<dyn KvBackend>) -> Self {
        Self {
            db,
            event_seq: AtomicI64::new(0),
            max_search_results: 0,
        }
    }

    /// The backend (shared with the block indexer).
    #[must_use]
    pub fn db(&self) -> &Arc<dyn KvBackend> {
        &self.db
    }

    /// `Get`.
    pub fn get(&self, hash: &[u8]) -> Result<Option<abci::TxResult>, String> {
        if hash.is_empty() {
            return Err("transaction hash cannot be empty".into());
        }
        match self.db.get(hash).map_err(kv_err)? {
            None => Ok(None),
            Some(bz) => abci::TxResult::decode(bz.as_slice())
                .map(Some)
                .map_err(|e| format!("error reading TxResult: {e}")),
        }
    }

    /// `AddBatch`.
    pub fn add_batch(&self, results: &[abci::TxResult]) -> Result<(), String> {
        let mut batch = WriteBatch::new();
        for r in results {
            let hash = tx_hash(&r.tx);
            let ok = r.result.as_ref().is_none_or(|x| x.code == 0);
            if !ok {
                if let Some(old) = self.get(&hash)? {
                    if old.result.as_ref().is_none_or(|x| x.code == 0) {
                        continue;
                    }
                }
            }
            self.index_events(r, &hash, &mut batch)?;
            batch.put(key_for_height(r), hash.clone());
            batch.put(hash, r.encode_to_vec());
        }
        self.db.write(batch).map_err(kv_err)
    }

    fn index_events(
        &self,
        r: &abci::TxResult,
        hash: &[u8],
        batch: &mut WriteBatch,
    ) -> Result<(), String> {
        let Some(res) = &r.result else { return Ok(()) };
        for ev in &res.events {
            let seq = self.event_seq.fetch_add(1, AtOrd::SeqCst) + 1;
            if ev.r#type.is_empty() {
                continue;
            }
            for a in &ev.attributes {
                if a.key.is_empty() {
                    continue;
                }
                let composite = format!("{}.{}", ev.r#type, a.key);
                if composite == TX_HASH_KEY || composite == TX_HEIGHT_KEY {
                    return Err(format!(
                        "event type and attribute key \"{composite}\" is reserved; please use a different key"
                    ));
                }
                if a.index {
                    batch.put(key_for_event(&composite, &a.value, r, seq), hash.to_vec());
                }
            }
        }
        Ok(())
    }

    fn set_tmp_hashes(&self, tmp: &mut HashMap<Vec<u8>, TxRef>, key: &[u8], value: &[u8]) {
        let seq = extract_event_seq_from_key(key);
        let r = match (extract_height_from_key(key), extract_index_from_key(key)) {
            (Ok(height), Ok(index)) => TxRef {
                hash: value.to_vec(),
                height,
                index,
            },
            _ => match self.get(value) {
                Ok(Some(res)) => TxRef {
                    hash: value.to_vec(),
                    height: res.height,
                    index: res.index,
                },
                _ => return,
            },
        };
        let mut k = value.to_vec();
        k.extend_from_slice(seq.as_bytes());
        tmp.insert(k, r);
    }

    fn intersect(
        filtered: HashMap<Vec<u8>, TxRef>,
        tmp: HashMap<Vec<u8>, TxRef>,
        first_run: bool,
    ) -> HashMap<Vec<u8>, TxRef> {
        if tmp.is_empty() || first_run {
            return tmp;
        }
        filtered
            .into_iter()
            .filter(|(k, v)| tmp.get(k).is_some_and(|t| t.hash == v.hash))
            .collect()
    }

    fn match_cond(
        &self,
        c: &Condition,
        start: &[u8],
        filtered: HashMap<Vec<u8>, TxRef>,
        first_run: bool,
        info: &HeightInfo,
    ) -> Result<HashMap<Vec<u8>, TxRef>, String> {
        if !first_run && filtered.is_empty() {
            return Ok(filtered);
        }
        let mut tmp = HashMap::new();
        match c.op {
            Token::Eq => {
                for (k, v) in iterate_prefix(self.db.as_ref(), start)? {
                    let Ok(h) = extract_height_from_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                    self.set_tmp_hashes(&mut tmp, &k, &v);
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            Token::Exists => {
                for (k, v) in iterate_prefix(self.db.as_ref(), &start_key(&[&c.tag]))? {
                    let Ok(h) = extract_height_from_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                    self.set_tmp_hashes(&mut tmp, &k, &v);
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            Token::Contains => {
                for (k, v) in iterate_prefix(self.db.as_ref(), &start_key(&[&c.tag]))? {
                    if !is_tag_key(&k) {
                        continue;
                    }
                    if extract_value_from_key(&k).contains(c.value()) {
                        let Ok(h) = extract_height_from_key(&k) else {
                            continue;
                        };
                        if !check_height_conditions(info, h).unwrap_or(false) {
                            continue;
                        }
                        self.set_tmp_hashes(&mut tmp, &k, &v);
                    }
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            _ => return Err("other operators should be handled already".into()),
        }
        Ok(Self::intersect(filtered, tmp, first_run))
    }

    fn match_range(
        &self,
        r: &QueryRange,
        start: &[u8],
        filtered: HashMap<Vec<u8>, TxRef>,
        first_run: bool,
        info: &HeightInfo,
    ) -> Result<HashMap<Vec<u8>, TxRef>, String> {
        if !first_run && filtered.is_empty() {
            return Ok(filtered);
        }
        let mut tmp = HashMap::new();
        for (k, v) in iterate_prefix(self.db.as_ref(), start)? {
            if !is_tag_key(&k) {
                continue;
            }
            if r.any_bound_is_number() {
                let value = extract_value_from_key(&k);
                let Some(num) = parse_index_number(&value) else {
                    continue;
                };
                if r.key != TX_HEIGHT_KEY {
                    let Ok(h) = extract_height_from_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                }
                if check_bounds(r, &num).unwrap_or(false) {
                    self.set_tmp_hashes(&mut tmp, &k, &v);
                }
            }
            if limit_reached(tmp.len(), self.max_search_results) {
                return Err(ERR_TOO_MANY_RESULTS.into());
            }
        }
        Ok(Self::intersect(filtered, tmp, first_run))
    }

    fn search_refs(&self, q: &Query) -> Result<HashMap<Vec<u8>, TxRef>, String> {
        // `lookForHash`.
        if let Some(c) = q.conds.iter().find(|c| c.tag == TX_HASH_KEY) {
            let hash = hex::decode(c.value()).map_err(|e| {
                format!(
                    "error during searching for a hash in the query: {}",
                    go_hex_error(c.value(), &e)
                )
            })?;
            return match self.get(&hash) {
                Err(e) => Err(format!("error while retrieving the result: {e}")),
                Ok(None) => Ok(HashMap::new()),
                Ok(Some(res)) => Ok(HashMap::from([(
                    hash.clone(),
                    TxRef {
                        hash,
                        height: res.height,
                        index: res.index,
                    },
                )])),
            };
        }
        let mut skip: Vec<usize> = Vec::new();
        let (conds, mut info, _) = dedup_height(&q.conds, TX_HEIGHT_KEY);
        if !info.only_height_eq {
            // Go appends `heightEqIdx` (-1 when absent).
            if let Some(i) = info.height_eq_idx {
                skip.push(i);
            }
        }
        let (ranges, range_idx, height_range) = look_for_ranges_with_height(&conds, TX_HEIGHT_KEY);
        info.height_range = height_range;
        let mut initialized = false;
        let mut filtered = HashMap::new();
        if !ranges.is_empty() {
            skip.extend(range_idx);
            for r in &ranges {
                if r.key == TX_HEIGHT_KEY && !info.only_height_range {
                    continue;
                }
                if initialized {
                    filtered =
                        self.match_range(r, &start_key(&[&r.key]), filtered, false, &info)?;
                } else {
                    filtered = self.match_range(r, &start_key(&[&r.key]), filtered, true, &info)?;
                    initialized = true;
                    if filtered.is_empty() {
                        break;
                    }
                }
            }
        }
        for (i, c) in conds.iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            let start = if info.height > 0 {
                start_key(&[&c.tag, c.value(), &info.height.to_string()])
            } else {
                start_key(&[&c.tag, c.value()])
            };
            if initialized {
                filtered = self.match_cond(c, &start, filtered, false, &info)?;
            } else {
                filtered = self.match_cond(c, &start, filtered, true, &info)?;
                initialized = true;
                if filtered.is_empty() {
                    break;
                }
            }
        }
        Ok(filtered)
    }

    /// `SearchPaged`: results of one page and the total count.
    pub fn search_paged(
        &self,
        q: &Query,
        order_by: &str,
        skip: usize,
        page_size: usize,
    ) -> Result<(Vec<abci::TxResult>, usize), String> {
        let filtered = self.search_refs(q)?;
        // `uniqueRefs`.
        let mut by_hash: HashMap<Vec<u8>, TxRef> = HashMap::new();
        for r in filtered.into_values() {
            match by_hash.get(&r.hash) {
                Some(e) if !tx_ref_less(e, &r) => {}
                _ => {
                    by_hash.insert(r.hash.clone(), r);
                }
            }
        }
        let mut refs: Vec<TxRef> = by_hash.into_values().collect();
        match order_by {
            "desc" => refs.sort_by(|a, b| (b.height, b.index).cmp(&(a.height, a.index))),
            "asc" | "" => refs.sort_by_key(|a| (a.height, a.index)),
            _ => return Err("expected order_by to be either `asc` or `desc` or empty".into()),
        }
        let total = refs.len();
        if skip >= total || page_size == 0 {
            return Ok((Vec::new(), total));
        }
        let end = skip.saturating_add(page_size).min(total);
        let mut out = Vec::new();
        for r in &refs[skip..end] {
            match self.get(&r.hash) {
                Err(e) => {
                    return Err(format!(
                        "failed to get Tx{{{}}}: {e}",
                        hex::encode_upper(&r.hash)
                    ));
                }
                Ok(None) => {}
                Ok(Some(res)) => out.push(res),
            }
        }
        Ok((out, total))
    }
}

fn parse_index_number(value: &str) -> Option<Dec> {
    // `big.Int.SetString(v, 10)` else `big.ParseFloat(v, 10, 125, …)`.
    Dec::parse_float(value)
}

/// Go's `hex.DecodeString` error text.
#[must_use]
pub fn go_hex_error(s: &str, e: &hex::FromHexError) -> String {
    match e {
        hex::FromHexError::InvalidHexCharacter { c, .. } => {
            let mut buf = [0u8; 4];
            let b = c.encode_utf8(&mut buf).as_bytes()[0];
            format!(
                "encoding/hex: invalid byte: U+{:04X} {}",
                b,
                go_rune_quote(b)
            )
        }
        hex::FromHexError::OddLength | hex::FromHexError::InvalidStringLength => {
            // Go reports an invalid byte before the odd length when one exists.
            for b in s.bytes() {
                if !b.is_ascii_hexdigit() {
                    return format!("encoding/hex: invalid byte: U+{b:04X} {}", go_rune_quote(b));
                }
            }
            "encoding/hex: odd length hex string".to_owned()
        }
    }
}

/// `%q` of a rune given as a byte (Go's `strconv.QuoteRune`).
fn go_rune_quote(b: u8) -> String {
    let c = char::from(b);
    match c {
        '\'' => "'\\''".to_owned(),
        '\\' => "'\\\\'".to_owned(),
        '\n' => "'\\n'".to_owned(),
        '\r' => "'\\r'".to_owned(),
        '\t' => "'\\t'".to_owned(),
        c if (c as u32) < 0x20 || c as u32 == 0x7f => format!("'\\x{:02x}'", c as u32),
        c if (0x80..0xa0).contains(&(c as u32)) => format!("'\\u{:04x}'", c as u32),
        c => format!("'{c}'"),
    }
}

// --- block indexer ---------------------------------------------------------

const BLOCK_PREFIX: &[u8] = b"block_events";

/// `orderedcode` string append.
fn oc_string(out: &mut Vec<u8>, s: &[u8]) {
    for &b in s {
        match b {
            0x00 => out.extend_from_slice(&[0x00, 0xff]),
            0xff => out.extend_from_slice(&[0xff, 0x00]),
            b => out.push(b),
        }
    }
    out.extend_from_slice(&[0x00, 0x01]);
}

const MSB: [u8; 8] = [0x00, 0x80, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc, 0xfe];

/// `orderedcode` int64 append.
fn oc_int64(out: &mut Vec<u8>, x: i64) {
    if (-64..64).contains(&x) {
        // `uint8(x) ^ 0x80`: the low byte of the two's complement.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        out.push((x as u8) ^ 0x80);
        return;
    }
    let neg = x < 0;
    let mut x = if neg { !x } else { x };
    let mut n: usize = 1;
    let mut buf = [0u8; 10];
    let mut i: usize = 9;
    while x > 0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            buf[i] = x as u8;
        }
        n += 1;
        i -= 1;
        x >>= 8;
    }
    let leading_ff = n > 7;
    if leading_ff {
        n -= 7;
    }
    if u32::from(buf[i + 1]) < (1u32 << (8 - n)) {
        n -= 1;
        i += 1;
    }
    buf[i] |= MSB[n];
    if leading_ff {
        i -= 1;
        buf[i] = 0xff;
    }
    if neg {
        for b in &mut buf[i..] {
            *b = !*b;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

/// `orderedcode` string parse: (value, rest).
fn oc_parse_string(s: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            0x00 => {
                match s.get(i + 1)? {
                    0x01 => return Some((out, &s[i + 2..])),
                    0xff => out.push(0x00),
                    _ => return None,
                }
                i += 2;
            }
            0xff => {
                if s.get(i + 1)? != &0x00 {
                    return None;
                }
                out.push(0xff);
                i += 2;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    None
}

/// `orderedcode` int64 parse.
fn oc_parse_int64(s: &[u8]) -> Option<(i64, &[u8])> {
    let mut dir: u8 = 0;
    let mut c = *s.first()?;
    if (0x40..0xc0).contains(&c) {
        #[allow(clippy::cast_possible_wrap)]
        return Some((i64::from((c ^ 0x80) as i8), &s[1..]));
    }
    let neg = c & 0x80 == 0;
    if neg {
        c = !c;
        dir = 0xff;
    }
    let mut s = s;
    let mut n = 0usize;
    if c == 0xff {
        if s.len() == 1 {
            return None;
        }
        s = &s[1..];
        c = s[0] ^ dir;
        if c > 0xc0 {
            return None;
        }
        n = 7;
    }
    let mut mask = 0x80u8;
    while mask != 0 && c & mask != 0 {
        c &= !mask;
        n += 1;
        mask >>= 1;
    }
    if s.len() < n {
        return None;
    }
    let mut x = i64::from(c);
    for b in &s[1..n] {
        x = (x << 8) | i64::from(b ^ dir);
    }
    if neg {
        x = !x;
    }
    Some((x, &s[n..]))
}

fn height_key(h: i64) -> Vec<u8> {
    let mut k = Vec::new();
    oc_string(&mut k, BLOCK_HEIGHT_KEY.as_bytes());
    oc_int64(&mut k, h);
    k
}

fn event_key(composite: &str, value: &str, height: i64, seq: i64) -> Vec<u8> {
    let mut k = Vec::new();
    oc_string(&mut k, composite.as_bytes());
    oc_string(&mut k, value.as_bytes());
    oc_int64(&mut k, height);
    oc_int64(&mut k, seq);
    k
}

/// (composite, value, height, rest) of an event key.
fn parse_event_key(k: &[u8]) -> Option<(Vec<u8>, Vec<u8>, i64, &[u8])> {
    let (ck, rest) = oc_parse_string(k)?;
    let (v, rest) = oc_parse_string(rest)?;
    let (h, rest) = oc_parse_int64(rest)?;
    Some((ck, v, h, rest))
}

fn parse_event_seq(k: &[u8]) -> i64 {
    let Some((_, _, _, rest)) = parse_event_key(k) else {
        return 0;
    };
    if rest.is_empty() {
        return 0;
    }
    match oc_parse_string(rest) {
        None => match oc_parse_int64(rest) {
            Some((seq, r)) if r.is_empty() => seq,
            _ => 0,
        },
        Some((_, r2)) if !r2.is_empty() => match oc_parse_int64(r2) {
            Some((seq, r)) if r.is_empty() => seq,
            _ => 0,
        },
        Some(_) => 0,
    }
}

fn varint_i64(v: i64) -> Vec<u8> {
    // Go `binary.PutVarint` (zig-zag).
    #[allow(clippy::cast_sign_loss)]
    let mut u = ((v << 1) ^ (v >> 63)) as u64;
    let mut out = Vec::new();
    while u >= 0x80 {
        #[allow(clippy::cast_possible_truncation)]
        out.push((u as u8) | 0x80);
        u >>= 7;
    }
    #[allow(clippy::cast_possible_truncation)]
    out.push(u as u8);
    out
}

fn varint_decode(b: &[u8]) -> i64 {
    let mut u: u64 = 0;
    let mut shift = 0;
    for &x in b {
        u |= u64::from(x & 0x7f) << shift;
        if x < 0x80 {
            break;
        }
        shift += 7;
        if shift > 63 {
            break;
        }
    }
    #[allow(clippy::cast_possible_wrap)]
    let r = ((u >> 1) as i64) ^ -((u & 1) as i64);
    r
}

/// `indexer/block/kv.BlockerIndexer`.
#[derive(Debug)]
pub struct BlockIndexer {
    db: Arc<dyn KvBackend>,
    event_seq: AtomicI64,
    /// `max_search_results` (0 = unlimited).
    pub max_search_results: usize,
}

fn prefixed(k: &[u8]) -> Vec<u8> {
    let mut v = BLOCK_PREFIX.to_vec();
    v.extend_from_slice(k);
    v
}

impl BlockIndexer {
    /// `New` over `NewPrefixDB(db, "block_events")`.
    #[must_use]
    pub fn new(db: Arc<dyn KvBackend>) -> Self {
        Self {
            db,
            event_seq: AtomicI64::new(0),
            max_search_results: 0,
        }
    }

    fn iter(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        Ok(iterate_prefix(self.db.as_ref(), &prefixed(prefix))?
            .into_iter()
            .map(|(k, v)| (k[BLOCK_PREFIX.len()..].to_vec(), v))
            .collect())
    }

    /// `Has`.
    pub fn has(&self, height: i64) -> Result<bool, String> {
        Ok(self
            .db
            .get(&prefixed(&height_key(height)))
            .map_err(kv_err)?
            .is_some())
    }

    /// `Index` (FinalizeBlock events of a height).
    pub fn index(&self, height: i64, events: &[abci::Event]) -> Result<(), String> {
        let mut batch = WriteBatch::new();
        let hb = varint_i64(height);
        batch.put(prefixed(&height_key(height)), hb.clone());
        for ev in events {
            let seq = self.event_seq.fetch_add(1, AtOrd::SeqCst) + 1;
            if ev.r#type.is_empty() {
                continue;
            }
            for a in &ev.attributes {
                if a.key.is_empty() {
                    continue;
                }
                let composite = format!("{}.{}", ev.r#type, a.key);
                if composite == BLOCK_HEIGHT_KEY {
                    return Err(format!(
                        "failed to index FinalizeBlock events: event type and attribute key \"{composite}\" is reserved; please use a different key"
                    ));
                }
                if a.index {
                    batch.put(
                        prefixed(&event_key(&composite, &a.value, height, seq)),
                        hb.clone(),
                    );
                }
            }
        }
        self.db.write(batch).map_err(kv_err)
    }

    fn set_tmp(tmp: &mut HashMap<Vec<u8>, Vec<u8>>, k: &[u8], v: &[u8]) {
        let seq = parse_event_seq(k);
        let mut key = v.to_vec();
        key.extend_from_slice(seq.to_string().as_bytes());
        tmp.insert(key, v.to_vec());
    }

    fn intersect(
        filtered: HashMap<Vec<u8>, Vec<u8>>,
        tmp: HashMap<Vec<u8>, Vec<u8>>,
        first_run: bool,
    ) -> HashMap<Vec<u8>, Vec<u8>> {
        if tmp.is_empty() || first_run {
            return tmp;
        }
        filtered
            .into_iter()
            .filter(|(k, v)| tmp.get(k).is_some_and(|t| t == v))
            .collect()
    }

    fn match_range(
        &self,
        r: &QueryRange,
        filtered: HashMap<Vec<u8>, Vec<u8>>,
        first_run: bool,
        info: &HeightInfo,
    ) -> Result<HashMap<Vec<u8>, Vec<u8>>, String> {
        if !first_run && filtered.is_empty() {
            return Ok(filtered);
        }
        let mut prefix = Vec::new();
        oc_string(&mut prefix, r.key.as_bytes());
        let mut tmp = HashMap::new();
        for (k, v) in self.iter(&prefix)? {
            let value = if r.key == BLOCK_HEIGHT_KEY {
                // `parseValueFromPrimaryKey`.
                match oc_parse_string(&k).and_then(|(_, rest)| oc_parse_int64(rest)) {
                    Some((h, rest)) if rest.is_empty() => h.to_string(),
                    _ => continue,
                }
            } else {
                match parse_event_key(&k) {
                    Some((_, v, _, _)) => String::from_utf8_lossy(&v).into_owned(),
                    None => continue,
                }
            };
            if r.any_bound_is_number() {
                let Some(num) = Dec::parse_float(&value) else {
                    continue;
                };
                if r.key != BLOCK_HEIGHT_KEY {
                    let Some((_, _, h, _)) = parse_event_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                }
                if check_bounds(r, &num).unwrap_or(false) {
                    Self::set_tmp(&mut tmp, &k, &v);
                }
            }
            if limit_reached(tmp.len(), self.max_search_results) {
                return Err(ERR_TOO_MANY_RESULTS.into());
            }
        }
        Ok(Self::intersect(filtered, tmp, first_run))
    }

    fn match_cond(
        &self,
        c: &Condition,
        filtered: HashMap<Vec<u8>, Vec<u8>>,
        first_run: bool,
        info: &HeightInfo,
    ) -> Result<HashMap<Vec<u8>, Vec<u8>>, String> {
        if !first_run && filtered.is_empty() {
            return Ok(filtered);
        }
        let mut tmp = HashMap::new();
        let mut prefix = Vec::new();
        oc_string(&mut prefix, c.tag.as_bytes());
        match c.op {
            Token::Eq => {
                oc_string(&mut prefix, c.value().as_bytes());
                for (k, v) in self.iter(&prefix)? {
                    let Some((_, _, h, _)) = parse_event_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                    Self::set_tmp(&mut tmp, &k, &v);
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            Token::Exists => {
                for (k, v) in self.iter(&prefix)? {
                    let Some((_, _, h, _)) = parse_event_key(&k) else {
                        continue;
                    };
                    if !check_height_conditions(info, h).unwrap_or(false) {
                        continue;
                    }
                    Self::set_tmp(&mut tmp, &k, &v);
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            Token::Contains => {
                for (k, v) in self.iter(&prefix)? {
                    let Some((_, value, h, _)) = parse_event_key(&k) else {
                        continue;
                    };
                    if String::from_utf8_lossy(&value).contains(c.value()) {
                        if !check_height_conditions(info, h).unwrap_or(false) {
                            continue;
                        }
                        Self::set_tmp(&mut tmp, &k, &v);
                    }
                    if limit_reached(tmp.len(), self.max_search_results) {
                        return Err(ERR_TOO_MANY_RESULTS.into());
                    }
                }
            }
            _ => return Err("other operators should be handled already".into()),
        }
        Ok(Self::intersect(filtered, tmp, first_run))
    }

    /// `Search`: matching heights in ascending order.
    pub fn search(&self, q: &Query) -> Result<Vec<i64>, String> {
        let (conds, mut info, found) = dedup_height(&q.conds, BLOCK_HEIGHT_KEY);
        let (ranges, range_idx, height_range) =
            look_for_ranges_with_height(&conds, BLOCK_HEIGHT_KEY);
        info.height_range = height_range;
        if found && info.only_height_eq {
            return Ok(if self.has(info.height)? {
                vec![info.height]
            } else {
                Vec::new()
            });
        }
        let mut skip: Vec<usize> = Vec::new();
        if let Some(i) = info.height_eq_idx {
            skip.push(i);
        }
        let mut initialized = false;
        let mut filtered = HashMap::new();
        if !ranges.is_empty() {
            skip.extend(range_idx);
            for r in &ranges {
                if r.key == BLOCK_HEIGHT_KEY && !info.only_height_range {
                    continue;
                }
                if initialized {
                    filtered = self.match_range(r, filtered, false, &info)?;
                } else {
                    filtered = self.match_range(r, filtered, true, &info)?;
                    initialized = true;
                    if filtered.is_empty() {
                        break;
                    }
                }
            }
        }
        for (i, c) in conds.iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            if initialized {
                filtered = self.match_cond(c, filtered, false, &info)?;
            } else {
                filtered = self.match_cond(c, filtered, true, &info)?;
                initialized = true;
                if filtered.is_empty() {
                    break;
                }
            }
        }
        let mut seen = BTreeMap::new();
        for hb in filtered.values() {
            let h = varint_decode(hb);
            if self.has(h)? {
                seen.insert(h, ());
            }
        }
        Ok(seen.into_keys().collect())
    }
}

/// Indexes one block: FinalizeBlock events into the block index, the tx
/// results into the tx index (`IndexerService`).
pub fn index_block(
    txi: &TxIndexer,
    bi: &BlockIndexer,
    height: i64,
    block_events: &[abci::Event],
    txs: &[abci::TxResult],
) -> Result<(), String> {
    bi.index(height, block_events)?;
    txi.add_batch(txs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orderedcode_ints_roundtrip() {
        for x in [
            0i64,
            1,
            -1,
            63,
            64,
            -64,
            -65,
            255,
            256,
            1 << 20,
            i64::MAX,
            i64::MIN,
            -300,
        ] {
            let mut b = Vec::new();
            oc_int64(&mut b, x);
            let (y, rest) = oc_parse_int64(&b).unwrap();
            assert_eq!((x, rest.len()), (y, 0), "{x}");
        }
        let mut b = Vec::new();
        oc_int64(&mut b, 0x10e);
        assert_eq!(b, vec![0xc1, 0x0e]);
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0i64, 1, -1, 300, i64::MAX] {
            assert_eq!(varint_decode(&varint_i64(v)), v);
        }
    }
}
