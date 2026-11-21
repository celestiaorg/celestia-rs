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
    validate_tx_order(&classified)?;
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

fn validate_tx_order(txs: &[ClassifiedTx]) -> Result<(), SdkError> {
    // The builder groups transactions by kind when resolving the share range
    // for a block transaction index.
    let (mut seen_blob, mut seen_fibre) = (false, false);
    for (index, tx) in txs.iter().enumerate() {
        match tx {
            ClassifiedTx::Fibre(_) => seen_fibre = true,
            ClassifiedTx::Blob(_) if seen_fibre => {
                return Err(plain(format!(
                    "blob tx at index {index} is after a pay-for-fibre tx"
                )));
            }
            ClassifiedTx::Blob(_) => seen_blob = true,
            ClassifiedTx::Normal(_) if seen_blob || seen_fibre => {
                return Err(plain(format!(
                    "normal tx at index {index} is after a blob or pay-for-fibre tx"
                )));
            }
            ClassifiedTx::Normal(_) => {}
        }
    }
    Ok(())
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

/// One ordered SHWAP namespace row. An empty share list carries a proof
/// of absence for a namespace lying between two leaves in the row.
#[cfg(feature = "bridge")]
pub struct NamespaceRow {
    /// Raw shares whose namespace equals the query.
    pub shares: Vec<Vec<u8>>,
    /// Inclusion range or first larger leaf and its authentication path.
    pub proof: pb::NmtProof,
}

/// Prove an end-exclusive range of cells against one EDS row, including parity cells.
#[cfg(feature = "bridge")]
pub fn row_range_proof(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    row: usize,
    start: usize,
    end: usize,
) -> Result<pb::NmtProof, SdkError> {
    let cells = eds.row(row).map_err(|e| square_err(&e))?;
    if start >= end || end > cells.len() {
        return Err(plain("column range out of bounds"));
    }
    let leaves: Vec<_> = cells
        .iter()
        .enumerate()
        .map(|(index, share)| Node::leaf(&nmt_leaf(eds.original_width(), row, index, share)))
        .collect();
    let mut nodes = Vec::new();
    let root = range_proof(&leaves, start, end, &mut nodes)?;
    if dah
        .row_roots
        .get(row)
        .is_none_or(|expected| expected.as_slice() != root.to_bytes())
    {
        return Err(plain("sample row does not match data availability header"));
    }
    Ok(pb::NmtProof {
        start: i32::try_from(start).map_err(|_| plain("invalid proof start"))?,
        end: i32::try_from(end).map_err(|_| plain("invalid proof end"))?,
        nodes: nodes.into_iter().map(Node::to_bytes).collect(),
        leaf_hash: Vec::new(),
    })
}

/// Prove every candidate row of an authenticated EDS, as required by
/// celestia-node's NamespaceData.Verify.
#[cfg(feature = "bridge")]
pub fn namespace_rows(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    namespace: Namespace,
) -> Result<Vec<NamespaceRow>, SdkError> {
    if namespace.is_parity_shares() || namespace.is_tail_padding() {
        return Err(plain("invalid data namespace"));
    }
    let mut result = Vec::new();
    for i in dah
        .rows_with_namespace(namespace)
        .map_err(|e| square_err(&e))?
    {
        let row = eds.row(i).map_err(|e| square_err(&e))?;
        let leaves: Vec<Node> = row
            .iter()
            .enumerate()
            .map(|(col, sh)| Node::leaf(&nmt_leaf(eds.original_width(), i, col, sh)))
            .collect();
        let ns = namespace.as_bytes();
        let start = leaves.partition_point(|leaf| &leaf.min < ns);
        let end = leaves.partition_point(|leaf| &leaf.min <= ns);
        let absent = start == end;
        let proof_end = if absent { start + 1 } else { end };
        let mut nodes = Vec::new();
        let root = range_proof(&leaves, start, proof_end, &mut nodes)?;
        if root.to_bytes().as_slice() != dah.row_roots[i] {
            return Err(plain(
                "namespace row does not match data availability header",
            ));
        }
        result.push(NamespaceRow {
            shares: row[start..end].to_vec(),
            proof: pb::NmtProof {
                start: i32::try_from(start).map_err(|_| plain("invalid proof start"))?,
                end: i32::try_from(proof_end).map_err(|_| plain("invalid proof end"))?,
                nodes: nodes.into_iter().map(Node::to_bytes).collect(),
                leaf_hash: if absent {
                    leaves[start].to_bytes()
                } else {
                    Vec::new()
                },
            },
        });
    }
    Ok(result)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "bridge")]
    #[test]
    fn shwap_namespace_ranges_cross_rows_and_prove_absence() {
        let ns = |n| Namespace::new_v0(&[1, n]).unwrap();
        let shares: Vec<_> = [1, 3, 3, 5]
            .into_iter()
            .map(|n| {
                let blob = shares::Blob::new_v0(ns(n), vec![n]).unwrap();
                shares::split_blobs(&[blob]).unwrap().remove(0)
            })
            .collect();
        let eds = square::extend_shares(&shares).unwrap();
        let dah = square::new_data_availability_header(&eds).unwrap();
        let found = namespace_rows(&eds, &dah, ns(3)).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].proof.start, found[0].proof.end), (1, 2));
        assert_eq!((found[1].proof.start, found[1].proof.end), (0, 1));
        assert_eq!(found[0].shares, [eds.share(0, 1).unwrap()]);
        assert_eq!(found[1].shares, [eds.share(1, 0).unwrap()]);
        let gap = namespace_rows(&eds, &dah, ns(2)).unwrap();
        assert_eq!(gap.len(), 1);
        assert!(gap[0].shares.is_empty());
        assert_eq!(gap[0].proof.leaf_hash.len(), square::DAH_ROOT_SIZE);
        assert_eq!(gap[0].proof.start, 1);
        assert!(namespace_rows(&eds, &dah, ns(6)).unwrap().is_empty());
    }

    use celestia_proto::Any;
    use celestia_proto::celestia::fibre::v1::{MsgPayForFibre, PaymentPromise};
    use celestia_proto::cosmos::crypto::secp256k1::PubKey;
    use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};
    use shares::{FIBRE_COMMITMENT_SIZE, NAMESPACE_VERSION_ZERO_ID_SIZE, SIGNER_SIZE};

    fn proof_transactions() -> [Vec<u8>; 3] {
        let encode_tx = |body: TxBody| {
            TxRaw {
                body_bytes: body.encode_to_vec(),
                ..Default::default()
            }
            .encode_to_vec()
        };
        let normal = encode_tx(TxBody {
            memo: "proof fixture".to_owned(),
            ..Default::default()
        });
        let blob_namespace =
            Namespace::new_v0(&[1; NAMESPACE_VERSION_ZERO_ID_SIZE]).expect("blob namespace");
        let blob = txdecode::marshal_blob_tx(
            &normal,
            &[sdk::tx::Blob {
                namespace_id: blob_namespace.id().to_vec(),
                namespace_version: 0,
                share_version: 0,
                data: b"proof fixture".to_vec(),
                signer: vec![],
            }],
        );
        let fibre_namespace =
            Namespace::new_v0(&[2; NAMESPACE_VERSION_ZERO_ID_SIZE]).expect("fibre namespace");
        let message = MsgPayForFibre {
            signer: encoding::bech32::encode("celestia", &[3; SIGNER_SIZE]).expect("signer"),
            payment_promise: Some(PaymentPromise {
                namespace: fibre_namespace.as_bytes().to_vec(),
                commitment: vec![4; FIBRE_COMMITMENT_SIZE],
                // The Go encoder includes these non-nullable fields.
                creation_timestamp: Some(pbjson_types::Timestamp {
                    seconds: -62_135_596_800,
                    nanos: 0,
                }),
                signer_public_key: Some(PubKey::default()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let fibre = encode_tx(TxBody {
            messages: vec![Any {
                type_url: crate::fibre_tx::pff_type_url(),
                value: message.encode_to_vec(),
            }],
            ..Default::default()
        });
        [normal, blob, fibre]
    }

    #[test]
    fn tx_proofs_match_go_v11_release() {
        let txs = proof_transactions();
        // SHA-256 of ShareProof.Marshal() on celestia-app v11.0.0-mocha and
        // v11.0.0-corto, both at 58be8ae9.
        for (index, namespace, expected) in [
            (
                0,
                TX_NAMESPACE,
                "22817e45819b3bffd9417047ad16d917a703ceceb42ed476ccc1d75bfcbd3291",
            ),
            (
                1,
                PAY_FOR_BLOB_NAMESPACE,
                "3c4ab05c9c79a017a7bda1729d31f8c7a9f75ce28b80d7ffc79941b51f1d7223",
            ),
            (
                2,
                PAY_FOR_FIBRE_NAMESPACE,
                "b498c987992eaa69c911f3f98f65389da52cf9305619fa3cb42def7e31e598fb",
            ),
        ] {
            let proof = new_tx_inclusion_proof(&txs, index).expect("ordered transaction proof");
            assert_eq!(proof.namespace_id.as_slice(), namespace.id());
            assert_eq!(hex::encode(hash::sha256(&proof.encode_to_vec())), expected);
        }
    }

    #[test]
    fn tx_proofs_require_square_transaction_order() {
        let [normal, blob, fibre] = proof_transactions();
        for (txs, expected) in [
            (
                [blob.clone(), normal.clone(), fibre.clone()],
                "normal tx at index 1 is after a blob or pay-for-fibre tx",
            ),
            (
                [fibre.clone(), normal.clone(), blob.clone()],
                "normal tx at index 1 is after a blob or pay-for-fibre tx",
            ),
            (
                [normal, fibre, blob],
                "blob tx at index 2 is after a pay-for-fibre tx",
            ),
        ] {
            let error = new_tx_inclusion_proof(&txs, 0).expect_err("unordered transactions");
            assert_eq!(error.log, expected);
        }
    }
}
