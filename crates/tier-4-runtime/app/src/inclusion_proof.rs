//! The `/custom/txInclusionProof/<index>` and
//! `/custom/shareInclusionProof/<begin>/<end>` queriers (celestia-app
//! `pkg/proof`): share and row inclusion proofs computed from the block
//! passed in the request data (stateless; `celestia.core.v1.proof.ShareProof`
//! marshalled in the response value).

use celestia_proto::celestia::core::v1::proof as pb;
use celestia_proto::celestia_core_proto::tendermint::types::Block;
use prost::Message;
use sdk::SdkError;
use sdk::error::internal;
use shares::{Namespace, PAY_FOR_BLOB_NAMESPACE, PAY_FOR_FIBRE_NAMESPACE, Share, TX_NAMESPACE};
use square::v4::{self, Builder, ClassifiedTx};

use crate::proposal::{SQUARE_SIZE_UPPER_BOUND, SUBTREE_ROOT_THRESHOLD, classify_txs};

/// `TxInclusionQueryPath`.
pub const TX_INCLUSION_QUERY_PATH: &str = "txInclusionProof";
/// `ShareInclusionQueryPath`.
pub const SHARE_INCLUSION_QUERY_PATH: &str = "shareInclusionProof";

/// `appconsts.SquareSizeUpperBound` as a width.
const MAX_SQUARE: usize = 512;
const _: () = assert!(MAX_SQUARE as u64 == SQUARE_SIZE_UPPER_BOUND);

/// `appconsts.DefaultUpperBoundMaxBytes` (`maxQueryDataSize`).
const MAX_QUERY_DATA_SIZE: usize = 128 * 1024 * 1024;

fn plain(log: impl Into<String>) -> SdkError {
    internal(log)
}

fn square_err(e: &square::Error) -> SdkError {
    plain(e.to_string())
}

/// `strconv.ParseInt(s, 10, 64)` with Go's error text.
fn parse_int(s: &str) -> Result<i64, SdkError> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(plain(format!(
            "strconv.ParseInt: parsing {s:?}: invalid syntax"
        )));
    }
    s.parse::<i64>().map_err(|_| {
        plain(format!(
            "strconv.ParseInt: parsing {s:?}: value out of range"
        ))
    })
}

fn check_size(data: &[u8]) -> Result<(), SdkError> {
    if data.len() > MAX_QUERY_DATA_SIZE {
        return Err(plain(format!(
            "request data too large: {} bytes exceeds maximum of {MAX_QUERY_DATA_SIZE} bytes",
            data.len()
        )));
    }
    Ok(())
}

fn read_block(data: &[u8]) -> Result<Block, SdkError> {
    Block::decode(data).map_err(|e| {
        plain(format!(
            "error reading block: {}",
            grpc_query::decode_error(&e).go_error_string()
        ))
    })
}

fn classify(txs: &[Vec<u8>]) -> Result<Vec<ClassifiedTx>, SdkError> {
    classify_txs(txs).map_err(plain)
}

/// `QueryTxInclusionProof`.
pub fn query_tx_inclusion_proof(path: &[&str], data: &[u8]) -> Result<Vec<u8>, SdkError> {
    check_size(data)?;
    if path.len() != 1 {
        return Err(plain(format!(
            "expected query path length: 1 actual: {} ",
            path.len()
        )));
    }
    let index = parse_int(path[0])?;
    if index < 0 {
        return Err(plain(format!(
            "path[0] element: {:?} produced a negative value: {index}",
            path[0]
        )));
    }
    let block = read_block(data)?;
    let txs = block.data.map(|d| d.txs).unwrap_or_default();
    let proof = new_tx_inclusion_proof(&txs, u64::try_from(index).unwrap_or_default())?;
    Ok(proof.encode_to_vec())
}

/// `NewTxInclusionProof`.
pub fn new_tx_inclusion_proof(txs: &[Vec<u8>], tx_index: u64) -> Result<pb::ShareProof, SdkError> {
    let index = usize::try_from(tx_index).unwrap_or(usize::MAX);
    if index >= txs.len() {
        return Err(plain(format!("txIndex {tx_index} out of bounds")));
    }
    let classified = classify(txs)?;
    let mut builder =
        Builder::new(MAX_SQUARE, SUBTREE_ROOT_THRESHOLD).map_err(|e| square_err(&e))?;
    for (idx, tx) in classified.iter().enumerate() {
        tx.validate()
            .map_err(|e| plain(format!("classified tx at index {idx}: {e}")))?;
        let added = match tx {
            ClassifiedTx::Fibre(f) => builder
                .append_fibre_tx(f)
                .map_err(|e| plain(format!("appending fibre tx at index {idx}: {e}")))?,
            ClassifiedTx::Blob(b) => builder
                .append_blob_tx(b)
                .map_err(|e| plain(format!("appending blob tx at index {idx}: {e}")))?,
            ClassifiedTx::Normal(t) => builder.append_tx(t),
        };
        if !added {
            let kind = match tx {
                ClassifiedTx::Fibre(_) => "fibre tx",
                ClassifiedTx::Blob(_) => "blob tx",
                ClassifiedTx::Normal(_) => "tx",
            };
            return Err(plain(format!(
                "not enough space to append {kind} at index {idx}"
            )));
        }
    }
    let data_square = builder.export().map_err(|e| square_err(&e))?;
    let (start, end) = builder
        .find_tx_share_range(index)
        .map_err(|e| square_err(&e))?;
    let namespace = match &classified[index] {
        ClassifiedTx::Fibre(_) => PAY_FOR_FIBRE_NAMESPACE,
        ClassifiedTx::Blob(_) => PAY_FOR_BLOB_NAMESPACE,
        ClassifiedTx::Normal(_) => TX_NAMESPACE,
    };
    new_share_inclusion_proof(data_square.as_shares(), namespace, start, end)
}

/// `QueryShareInclusionProof`.
pub fn query_share_inclusion_proof(path: &[&str], data: &[u8]) -> Result<Vec<u8>, SdkError> {
    check_size(data)?;
    if path.len() != 2 {
        return Err(plain(format!(
            "expected query path length: 2 actual: {} ",
            path.len()
        )));
    }
    let begin = parse_int(path[0])?;
    let end = parse_int(path[1])?;
    let block = read_block(data)?;
    let txs = block.data.map(|d| d.txs).unwrap_or_default();
    let classified = classify(&txs)?;
    let data_square = v4::construct(&classified, MAX_SQUARE, SUBTREE_ROOT_THRESHOLD)
        .map_err(|e| square_err(&e))?;
    let shares = data_square.as_shares();
    let namespace = parse_namespace(shares, begin, end)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let proof = new_share_inclusion_proof(shares, namespace, begin as usize, end as usize)?;
    Ok(proof.encode_to_vec())
}

/// `ParseNamespace`: the single namespace of the share range.
fn parse_namespace(shares: &[Share], start: i64, end: i64) -> Result<Namespace, SdkError> {
    if start < 0 {
        return Err(plain(format!("start share {start} should be positive")));
    }
    if end < 0 {
        return Err(plain(format!("end share {end} should be positive")));
    }
    if end <= start {
        return Err(plain(format!(
            "end share {end} cannot be lower or equal to the starting share {start}"
        )));
    }
    let (s, e) = (
        usize::try_from(start).unwrap_or(usize::MAX),
        usize::try_from(end).unwrap_or(usize::MAX),
    );
    if e > shares.len() {
        return Err(plain(format!(
            "end share {end} is higher than block shares {}",
            shares.len()
        )));
    }
    let ns = |sh: &Share| sh.namespace().map_err(|err| plain(err.to_string()));
    let first = ns(&shares[s])?;
    for (i, sh) in shares[s..e].iter().enumerate() {
        let other = ns(sh)?;
        if other != first {
            return Err(plain(format!(
                "shares range contain different namespaces at index {i}: {} and {} ",
                lower_hex(first.as_bytes()),
                lower_hex(other.as_bytes())
            )));
        }
    }
    Ok(first)
}

fn lower_hex(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// The NMT leaf of a share at `col` of row `row` in a square of width
/// `size` (`wrapper.ErasuredNamespacedMerkleTree.Push`).
fn nmt_leaf(size: usize, row: usize, col: usize, share: &[u8]) -> Vec<u8> {
    let mut leaf = Vec::with_capacity(shares::NAMESPACE_SIZE + share.len());
    if row < size && col < size {
        leaf.extend_from_slice(&share[..shares::NAMESPACE_SIZE]);
    } else {
        leaf.extend_from_slice(shares::PARITY_SHARES_NAMESPACE.as_bytes());
    }
    leaf.extend_from_slice(share);
    leaf
}

/// A node of celestia's namespaced Merkle tree (29-byte namespaces,
/// `IgnoreMaxNamespace`).
#[derive(Clone, Copy)]
struct Node {
    min: [u8; shares::NAMESPACE_SIZE],
    max: [u8; shares::NAMESPACE_SIZE],
    digest: [u8; 32],
}

impl Node {
    fn leaf(data: &[u8]) -> Self {
        let mut ns = [0u8; shares::NAMESPACE_SIZE];
        ns.copy_from_slice(&data[..shares::NAMESPACE_SIZE]);
        let mut h = hash::Sha256Hasher::new();
        h.update(&[0]);
        h.update(data);
        Self {
            min: ns,
            max: ns,
            digest: h.finalize(),
        }
    }

    fn parent(left: &Self, right: &Self) -> Self {
        let max = if right.min == *shares::PARITY_SHARES_NAMESPACE.as_bytes() {
            left.max
        } else {
            right.max
        };
        let mut h = hash::Sha256Hasher::new();
        h.update(&[1]);
        h.update(&left.to_bytes());
        h.update(&right.to_bytes());
        Self {
            min: left.min,
            max,
            digest: h.finalize(),
        }
    }

    fn to_bytes(self) -> Vec<u8> {
        [self.min.as_slice(), &self.max, &self.digest].concat()
    }
}

/// nmt `getSplitPoint`.
fn split_point(len: usize) -> usize {
    let mut k = 1usize << (usize::BITS - 1 - len.leading_zeros());
    if k == len {
        k >>= 1;
    }
    k
}

/// nmt `buildRangeProof` for `[start, end)`; returns the root and appends
/// the proof nodes.
fn range_proof(
    leaves: &[Node],
    start: usize,
    end: usize,
    proof: &mut Vec<Node>,
) -> Result<Node, SdkError> {
    fn recurse(
        leaves: &[Node],
        lo: usize,
        hi: usize,
        include: bool,
        (start, end): (usize, usize),
        proof: &mut Vec<Node>,
    ) -> Option<Node> {
        if lo >= leaves.len() {
            return None;
        }
        if hi - lo == 1 {
            let leaf = leaves[lo];
            if (lo < start || lo >= end) && include {
                proof.push(leaf);
            }
            return Some(leaf);
        }
        let new_include = include && !(hi <= start || lo >= end);
        let k = split_point(hi - lo);
        let left = recurse(leaves, lo, lo + k, new_include, (start, end), proof);
        let right = recurse(leaves, lo + k, hi, new_include, (start, end), proof);
        let hash = match (left, right) {
            (Some(l), Some(r)) => Node::parent(&l, &r),
            (l, None) => l?,
            (None, Some(r)) => r,
        };
        if include && !new_include {
            proof.push(hash);
        }
        Some(hash)
    }
    if leaves.is_empty() || start >= end || end > leaves.len() {
        return Err(plain("invalid proof range"));
    }
    let full = (split_point(leaves.len().max(2)) * 2).max(1);
    recurse(leaves, 0, full, true, (start, end), proof).ok_or_else(|| plain("empty tree"))
}

/// `NewShareInclusionProof` / `NewShareInclusionProofFromEDS` for the
/// end-exclusive share range `[start, end)`.
pub fn new_share_inclusion_proof(
    ods: &[Share],
    namespace: Namespace,
    start: usize,
    end: usize,
) -> Result<pb::ShareProof, SdkError> {
    let eds = square::extend_shares(ods).map_err(|e| square_err(&e))?;
    let size = square::square::size(ods.len());
    if size == 0 || end == 0 || end <= start {
        return Err(plain("invalid share range"));
    }
    let (start_row, end_row) = (start / size, (end - 1) / size);
    let (start_leaf, end_leaf) = (start % size, (end - 1) % size);
    let row_roots = eds.row_roots().map_err(|e| square_err(&e))?;
    let col_roots = eds.column_roots().map_err(|e| square_err(&e))?;
    let all: Vec<Vec<u8>> = row_roots
        .iter()
        .chain(col_roots.iter())
        .map(|r| r.to_vec())
        .collect();
    let (_, all_proofs) = merkle::proofs_from_byte_slices(&all);
    let mut row_proofs = Vec::new();
    let mut roots = Vec::new();
    for i in start_row..=end_row {
        let p = &all_proofs[i];
        row_proofs.push(pb::Proof {
            total: i64::try_from(p.total).unwrap_or(i64::MAX),
            index: i64::try_from(p.index).unwrap_or(i64::MAX),
            leaf_hash: p.leaf_hash.to_vec(),
            aunts: p.aunts.iter().map(|a| a.to_vec()).collect(),
        });
        roots.push(row_roots[i].to_vec());
    }
    let mut share_proofs = Vec::new();
    let mut raw_shares = Vec::new();
    let last = end_row - start_row;
    for (i, root) in roots.iter().enumerate() {
        let row = eds.row(start_row + i).map_err(|e| square_err(&e))?;
        // Go builds each tree with the row's position in the proved range
        // (not in the square) as its axis index.
        let leaves: Vec<Node> = row
            .iter()
            .enumerate()
            .map(|(col, sh)| Node::leaf(&nmt_leaf(size, i, col, sh)))
            .collect();
        let from = if i > 0 { 0 } else { start_leaf };
        let to = if i == last { end_leaf } else { size - 1 };
        let mut nodes = Vec::new();
        let tree_root = range_proof(&leaves, from, to + 1, &mut nodes)?;
        if tree_root.to_bytes().as_slice() != root.as_slice() {
            return Err(plain("eds row root is different than tree root"));
        }
        raw_shares.extend(row[from..=to].iter().cloned());
        share_proofs.push(pb::NmtProof {
            start: i32::try_from(from).unwrap_or(i32::MAX),
            end: i32::try_from(to + 1).unwrap_or(i32::MAX),
            nodes: nodes.iter().map(|n| n.to_bytes()).collect(),
            leaf_hash: Vec::new(),
        });
    }
    Ok(pb::ShareProof {
        data: raw_shares,
        share_proofs,
        namespace_id: namespace.id().to_vec(),
        row_proof: Some(pb::RowProof {
            row_roots: roots,
            proofs: row_proofs,
            root: Vec::new(),
            start_row: u32::try_from(start_row).unwrap_or(u32::MAX),
            end_row: u32::try_from(end_row).unwrap_or(u32::MAX),
        }),
        namespace_version: u32::from(namespace.version()),
    })
}
