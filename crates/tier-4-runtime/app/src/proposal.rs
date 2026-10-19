//! `PrepareProposal` / `ProcessProposal` (celestia-app v10
//! `app/prepare_proposal.go`, `app/filtered_square_builder.go`,
//! `app/process_proposal.go`) on top of cosmos-sdk `BaseApp`'s proposal
//! state handling.
//!
//! Both run on a branch of the latest state that is always discarded:
//! committed state, plus the InitChain writes at the initial height
//! (`getContextForProposal`). The ante handler is called directly on that
//! branch, as Go does, so a failing decorator's earlier writes stay visible
//! to later transactions.
//!
//! The `RequestProcessProposal.header` celestia-core sends is not read by
//! v10 (BaseApp builds the header from the request fields and the handler
//! only logs it).
//!
//! `MsgPayForFibre` promises are settled, and fibre escrow messages
//! replayed, on the proposal branch (`executeTxMsgs`) through the x/fibre
//! Msg server.

use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_proto::celestia_core_proto::abci::{
    RequestPrepareProposal, RequestProcessProposal, ResponsePrepareProposal,
    ResponseProcessProposal, response_process_proposal::ProposalStatus,
};
use prost::Name;
use sdk::tx::Tx;
use sdk::{Context, ExecMode, Header, SdkError};
use square::v4::{self, BlobTx, Builder, ClassifiedTx};
use txdecode::UnwrappedBlobTx;

use crate::msg_count::{MAX_SDK_MESSAGES, count_executable_msgs};
use crate::{App, fibre_tx, router};

/// `appconsts.MaxTxSize`.
pub const MAX_TX_SIZE: usize = 8_388_608;
/// `appconsts.MaxPFBMessages`.
pub const MAX_PFB_MESSAGES: usize = 200;
/// `appconsts.MaxPayForFibreMessages`.
pub const MAX_PAY_FOR_FIBRE_MESSAGES: usize = 200;
/// `appconsts.SubtreeRootThreshold`.
pub const SUBTREE_ROOT_THRESHOLD: usize = 64;
/// `appconsts.SquareSizeUpperBound`.
pub const SQUARE_SIZE_UPPER_BOUND: u64 = 512;

/// A Go panic (or handler error) that aborts the whole handler.
#[derive(Debug)]
struct Abort(#[allow(dead_code)] String);

impl From<SdkError> for Abort {
    fn from(e: SdkError) -> Self {
        Self(e.log)
    }
}

impl From<square::Error> for Abort {
    fn from(e: square::Error) -> Self {
        Self(e.to_string())
    }
}

/// Converts a decoded `BlobTx` into go-square's representation.
pub fn to_square_blob_tx(u: &UnwrappedBlobTx) -> Result<BlobTx, String> {
    let blobs = u
        .blobs
        .iter()
        .map(to_share_blob)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BlobTx {
        tx: u.tx.clone(),
        blobs,
    })
}

/// `sdk::tx::Blob` (already validated by `UnmarshalBlobTx`) as a
/// `shares::Blob`.
pub fn to_share_blob(b: &sdk::tx::Blob) -> Result<shares::Blob, String> {
    let mut ns = Vec::with_capacity(shares::NAMESPACE_SIZE);
    ns.push(u8::try_from(b.namespace_version).map_err(|e| e.to_string())?);
    ns.extend_from_slice(&b.namespace_id);
    let ns = shares::Namespace::from_slice(&ns).map_err(|e| e.to_string())?;
    let signer = if b.signer.is_empty() {
        None
    } else {
        Some(
            <[u8; shares::SIGNER_SIZE]>::try_from(b.signer.as_slice())
                .map_err(|e| e.to_string())?,
        )
    };
    shares::Blob::new_with_signer(
        ns,
        b.data.clone(),
        u8::try_from(b.share_version).map_err(|e| e.to_string())?,
        signer,
    )
    .map_err(|e| e.to_string())
}

/// `ValidateBlobTx` on a decoded `BlobTx` (`inner` is its decoded SDK tx).
pub fn validate_blob_tx(inner: &Tx, u: &UnwrappedBlobTx) -> sdk::Result<()> {
    let pfb = x_blob::pfb_from_msgs(inner.gogo_msgs())?;
    let blobs = u
        .blobs
        .iter()
        .map(to_share_blob)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| SdkError::new("undefined", 1, e))?;
    x_blob::validate_blob_tx(&pfb, &blobs, true)
}

/// `MaxEffectiveSquareSize`.
fn max_effective_square_size(ctx: &mut Context<'_>) -> Result<usize, Abort> {
    let gov = x_blob::get_params(ctx)?.gov_max_square_size;
    let size = gov.min(SQUARE_SIZE_UPPER_BOUND);
    usize::try_from(size).map_err(|e| Abort(e.to_string()))
}

/// `executeTxMsgs`: runs a tx's messages on a branch written only if all
/// succeed.
fn execute_tx_msgs(ctx: &mut Context<'_>, tx: &Tx) -> sdk::Result<()> {
    let outer_events = std::mem::take(&mut ctx.events);
    let cp = ctx.branch();
    // `defer recover()`: a handler panic (out of gas included) fails the tx.
    let res = sdk::error::recover(|| {
        for msg in tx.msgs() {
            if !router::has_handler(&msg.type_url) {
                return Err(sdk::error::sdk::unknown_request(format!(
                    "no message handler found for {}",
                    msg.type_url
                )));
            }
            router::route(ctx, msg)?;
        }
        Ok(())
    });
    if res.is_ok() {
        ctx.write(cp);
    } else {
        ctx.multistore().discard_branch();
    }
    ctx.events = outer_events;
    res
}

fn has_pfb(tx: &Tx) -> bool {
    let url = MsgPayForBlobs::type_url();
    tx.msgs().iter().any(|m| m.type_url == url)
}

/// `coretypes.ComputeProtoSizeForTxs([]Tx{tx})`.
fn proto_size(tx: &[u8]) -> i64 {
    let n = 1 + encoding::varint::encode_uvarint(tx.len() as u64).len() + tx.len();
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `ClassifyTxs`, with go-square's blob-tx unmarshalling done here.
pub(crate) fn classify_txs(txs: &[Vec<u8>]) -> Result<Vec<ClassifiedTx>, String> {
    txs.iter()
        .map(|raw| {
            if let Some(f) = fibre_tx::try_parse(raw)? {
                return Ok(ClassifiedTx::Fibre(f));
            }
            match txdecode::unmarshal_blob_tx(raw) {
                Some(Ok(u)) => Ok(ClassifiedTx::Blob(to_square_blob_tx(&u)?)),
                Some(Err(e)) => Err(format!("unmarshalling blob tx: {e:?}")),
                None => Ok(ClassifiedTx::Normal(raw.clone())),
            }
        })
        .collect()
}

/// go-square v4.0.0-rc4 `populateBuilder`'s classification (app version 9):
/// a `BlobTx` (an invalid one fails), else a pay-for-fibre tx by rc4's
/// `TryParseFibreTx` (a malformed one fails), else a normal tx. A v9 block
/// cannot hold a decodable `MsgPayForFibre` tx, but rc4 parses txs its own
/// way and lays out a tx whose repeated `body_bytes` put a `MsgPayForFibre`
/// first as a fibre tx ([`fibre_tx::try_parse_v9`]).
pub(crate) fn classify_txs_v9(txs: &[Vec<u8>]) -> Result<Vec<ClassifiedTx>, String> {
    txs.iter()
        .enumerate()
        .map(|(i, raw)| match txdecode::unmarshal_blob_tx_v9(raw) {
            Some(Ok(u)) => to_square_blob_tx(&u).map(ClassifiedTx::Blob),
            Some(Err(e)) => Err(format!("unmarshalling blob tx at index {i}: {e:?}")),
            None => match fibre_tx::try_parse_v9(raw) {
                Ok(Some(f)) => Ok(ClassifiedTx::Fibre(f)),
                Ok(None) => Ok(ClassifiedTx::Normal(raw.clone())),
                Err(e) => Err(format!("parsing fibre tx at index {i}: {e}")),
            },
        })
        .collect()
}

/// The square size and data root of a block's txs (`da.ConstructEDS` with
/// `SquareSizeUpperBound`, as celestia-app's proof and DAH helpers build
/// it): go-square rc4 classification at app version 9, the v10 one
/// otherwise.
pub fn block_data_root(app_version: u64, txs: &[Vec<u8>]) -> Result<(u64, Vec<u8>), String> {
    let size = usize::try_from(SQUARE_SIZE_UPPER_BOUND).map_err(|e| e.to_string())?;
    block_data_root_with_max(app_version, txs, size)
}

/// [`block_data_root`] with an explicit max square size
/// (`da.ConstructEDS(txs, appVersion, maxSquareSize)`).
pub fn block_data_root_with_max(
    app_version: u64,
    txs: &[Vec<u8>],
    max_square_size: usize,
) -> Result<(u64, Vec<u8>), String> {
    let square = if crate::stores::is_v9(app_version) {
        v4::construct_rc4(
            &classify_txs_v9(txs)?,
            max_square_size,
            SUBTREE_ROOT_THRESHOLD,
        )
    } else {
        v4::construct(&classify_txs(txs)?, max_square_size, SUBTREE_ROOT_THRESHOLD)
    }
    .map_err(|e| e.to_string())?;
    let (width, root) = data_root(&square).map_err(|e| e.0)?;
    Ok((width as u64, root))
}

/// Erasure-extends `square` and returns `(original width, data root)`.
fn data_root(square: &square::Square) -> Result<(usize, Vec<u8>), Abort> {
    let eds = square::extend_shares(square.as_shares())?;
    let dah = square::new_data_availability_header(&eds)?;
    Ok((eds.width() / 2, dah.hash().to_vec()))
}

/// The square this node built in its last PrepareProposal, with its
/// [`data_root`]. The data root is a pure function of the square's shares,
/// so ProcessProposal of a proposal whose (independently constructed)
/// square is byte-identical reuses it instead of erasure-extending the
/// square again: the proposer's own proposal skips the second EDS + DAH.
/// Any other square is extended as before, so acceptance is unchanged.
#[derive(Debug, Default)]
pub(crate) struct ProposedSquare(Option<(square::Square, usize, Vec<u8>)>);

impl ProposedSquare {
    /// [`data_root`] of `square`, from the cache when it holds this square.
    fn data_root(&self, square: &square::Square) -> Result<(usize, Vec<u8>), Abort> {
        match &self.0 {
            Some((cached, width, root)) if cached == square => Ok((*width, root.clone())),
            _ => data_root(square),
        }
    }
}

fn ante(ctx: &mut Context<'_>, tx: &Tx, deps: &x_ante::StoreDeps) -> Result<(), x_ante::AnteError> {
    x_ante::ante_handle_in_place(ctx, tx, false, deps).map(|_| ())
}

/// The ante handler as `FilteredSquareBuilder.Fill` runs it: celestia's
/// `HandlePanicDecorator` re-panics, and nothing in `Fill` recovers, so a
/// Go panic in the ante chain aborts the whole PrepareProposal (`BaseApp`
/// then returns the request's txs unchanged).
fn ante_fill(
    ctx: &mut Context<'_>,
    tx: &Tx,
    deps: &x_ante::StoreDeps,
) -> Result<Result<(), x_ante::AnteError>, Abort> {
    match ante(ctx, tx, deps) {
        Err(e) if e.panicked => Err(Abort(e.error.log)),
        r => Ok(r),
    }
}

impl App {
    /// The header `BaseApp` gives the proposal state: at the initial height
    /// the InitChain context with its header cleared (the chain id stays).
    fn proposal_header(
        &self,
        height: i64,
        time: Option<pbjson_types::Timestamp>,
        proposer: &[u8],
        next_validators_hash: &[u8],
    ) -> Header {
        let mut h = Header {
            chain_id: self.chain_id.clone(),
            height,
            time: time.unwrap_or_default(),
            proposer_address: proposer.to_vec(),
            ..Header::default()
        };
        if height != self.initial_height {
            h.app_hash.clone_from(&self.last_commit_hash);
            h.next_validators_hash = next_validators_hash.to_vec();
        }
        h
    }

    /// Runs `f` on a discarded branch of the proposal state.
    fn with_proposal_ctx<T>(
        &mut self,
        header: Header,
        mode: ExecMode,
        f: impl FnOnce(&mut Context<'_>, &x_ante::StoreDeps) -> T,
    ) -> T {
        let deps = x_ante::StoreDeps::new(None);
        let mut ctx = Context::new(&mut self.ms, header, mode);
        let base = ctx.multistore().depth();
        ctx.multistore().branch();
        ctx.consensus_params = x_consensus::get_params(&mut ctx)
            .ok()
            .flatten()
            .unwrap_or_default();
        sdk::version::set_app_version(ctx.consensus_params.version.as_ref().map_or(0, |v| v.app));
        let out = f(&mut ctx, &deps);
        while ctx.multistore().depth() > base {
            ctx.multistore().discard_branch();
        }
        out
    }

    /// `PrepareProposal`. Errors and panics of the handler make `BaseApp`
    /// return the request's txs unchanged (no square size, no data root).
    pub fn prepare_proposal(
        &mut self,
        req: &RequestPrepareProposal,
    ) -> sdk::Result<ResponsePrepareProposal> {
        if req.height < 1 {
            return Err(sdk::error::sdk::invalid_request(
                "PrepareProposal called with invalid height",
            ));
        }
        let header = self.proposal_header(
            req.height,
            req.time,
            &req.proposer_address,
            &req.next_validators_hash,
        );
        let mut proposed = std::mem::take(&mut self.proposed);
        proposed.0 = None;
        let res = self.with_proposal_ctx(header, ExecMode::PrepareProposal, |ctx, deps| {
            prepare(ctx, deps, req, &mut proposed)
        });
        self.proposed = proposed;
        Ok(res.unwrap_or_else(|_| ResponsePrepareProposal {
            txs: req.txs.clone(),
            ..ResponsePrepareProposal::default()
        }))
    }

    /// `ProcessProposal`.
    pub fn process_proposal(
        &mut self,
        req: &RequestProcessProposal,
    ) -> sdk::Result<ResponseProcessProposal> {
        if req.height < 1 {
            return Err(sdk::error::sdk::invalid_request(
                "ProcessProposal called with invalid height",
            ));
        }
        let header = self.proposal_header(
            req.height,
            req.time,
            &req.proposer_address,
            &req.next_validators_hash,
        );
        let proposed = std::mem::take(&mut self.proposed);
        let accept = self
            .with_proposal_ctx(header, ExecMode::ProcessProposal, |ctx, deps| {
                process(ctx, deps, req, &proposed)
            })
            .unwrap_or(false);
        self.proposed = proposed;
        Ok(ResponseProcessProposal {
            status: if accept {
                ProposalStatus::Accept
            } else {
                ProposalStatus::Reject
            }
            .into(),
        })
    }
}

/// The SDK txs of a proposal, split like `separateTxs`.
#[derive(Default)]
struct Separated<'a> {
    /// With their decoding (`fill` would decode them again).
    normal: Vec<(&'a [u8], Tx)>,
    blob: Vec<(&'a [u8], UnwrappedBlobTx)>,
    pff: Vec<&'a [u8]>,
}

/// `separateTxs`.
fn separate<'a>(txs: impl Iterator<Item = &'a [u8]>) -> Separated<'a> {
    let mut s = Separated::default();
    for raw in txs {
        if raw.len() > MAX_TX_SIZE {
            continue;
        }
        match txdecode::unmarshal_blob_tx(raw) {
            Some(Ok(u)) => s.blob.push((raw, u)),
            Some(Err(_)) => {}
            None => {
                let Ok(tx) = txdecode::decode(raw) else {
                    continue;
                };
                if fibre_tx::count_pff(tx.msgs()) > 0 {
                    if fibre_tx::validate_shape(&tx).is_ok() {
                        s.pff.push(raw);
                    }
                    continue;
                }
                s.normal.push((raw, tx));
            }
        }
    }
    s
}

/// `PrepareProposalHandler`.
fn prepare(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    req: &RequestPrepareProposal,
    proposed: &mut ProposedSquare,
) -> Result<ResponsePrepareProposal, Abort> {
    let max_square_size = max_effective_square_size(ctx)?;
    let mut builder = Builder::new(max_square_size, SUBTREE_ROOT_THRESHOLD)?;
    let txs = if x_ante::is_v9(ctx) {
        fill_v9(ctx, deps, &mut builder, &req.txs, req.max_tx_bytes)?
    } else {
        x_fibre::begin_block(ctx)?;
        fill(ctx, deps, &mut builder, &req.txs, req.max_tx_bytes)?
    };
    let square = builder.export()?;
    let (width, root) = data_root(&square)?;
    proposed.0 = Some((square, width, root.clone()));
    Ok(ResponsePrepareProposal {
        txs,
        square_size: width as u64,
        data_root_hash: root,
    })
}

/// `FilteredSquareBuilder.Fill`.
fn fill(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    builder: &mut Builder,
    txs: &[Vec<u8>],
    max_tx_bytes: i64,
) -> Result<Vec<Vec<u8>>, Abort> {
    let mut within_bytes: Vec<&[u8]> = Vec::with_capacity(txs.len());
    let mut current = 0i64;
    for tx in txs {
        if max_tx_bytes > 0 {
            let size = proto_size(tx);
            if current + size > max_tx_bytes {
                continue;
            }
            current += size;
        }
        within_bytes.push(tx);
    }
    let sep = separate(within_bytes.into_iter());

    // Set once an ante decorator returned `sdk.Context{}`: Go keeps using
    // that store-less context, so every later ante run fails and any state
    // read outside a recover panics the whole handler.
    let mut zeroed = false;
    let mut kept = Vec::new();

    let mut sdk_count = 0usize;
    for (raw, tx) in &sep.normal {
        ctx.tx_bytes = raw.to_vec();
        let exec = count_executable_msgs((!zeroed).then_some(&mut *ctx), tx.msgs())
            .map_err(|_| Abort("nil multistore".into()))?;
        if sdk_count + exec > MAX_SDK_MESSAGES {
            continue;
        }
        if !builder.append_tx(raw) {
            continue;
        }
        if zeroed {
            let _ = builder.revert_last_tx();
            continue;
        }
        if let Err(e) = ante_fill(ctx, tx, deps)? {
            zeroed |= e.zero_ctx;
            let _ = builder.revert_last_tx();
            continue;
        }
        if fibre_tx::contains_fibre_state_msg(tx) {
            let _ = execute_tx_msgs(ctx, tx);
        }
        sdk_count += exec;
        kept.push(raw.to_vec());
    }

    let mut blob_msgs = 0usize;
    for (raw, u) in &sep.blob {
        let Ok(tx) = txdecode::decode(&u.tx) else {
            continue;
        };
        ctx.tx_bytes.clone_from(&u.tx);
        if blob_msgs + tx.msgs().len() > MAX_PFB_MESSAGES {
            continue;
        }
        let Ok(btx) = to_square_blob_tx(u) else {
            continue;
        };
        match builder.append_blob_tx(&btx) {
            Ok(true) => {}
            Ok(false) | Err(_) => continue,
        }
        if zeroed {
            let _ = builder.revert_last_blob_tx();
            continue;
        }
        if let Err(e) = ante_fill(ctx, &tx, deps)? {
            zeroed |= e.zero_ctx;
            let _ = builder.revert_last_blob_tx();
            continue;
        }
        blob_msgs += tx.msgs().len();
        kept.push(raw.to_vec());
    }

    kept.extend(process_fibre_txs(ctx, deps, builder, &sep.pff, zeroed)?);
    Ok(kept)
}

/// celestia-app v9.0.8 `FilteredSquareBuilder.Fill` (app version 9): txs
/// sized by `len(tx)`, go-square rc4 blob unmarshalling, duplicate-field
/// txs dropped, `len(msgs)` message counts, blob txs re-marshaled.
fn fill_v9(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    builder: &mut Builder,
    txs: &[Vec<u8>],
    max_tx_bytes: i64,
) -> Result<Vec<Vec<u8>>, Abort> {
    let mut within_bytes: Vec<&[u8]> = Vec::with_capacity(txs.len());
    let mut current = 0i64;
    for tx in txs {
        if max_tx_bytes > 0 {
            let size = i64::try_from(tx.len()).unwrap_or(i64::MAX);
            if current + size > max_tx_bytes {
                continue;
            }
            current += size;
        }
        within_bytes.push(tx);
    }
    // separateTxs (v9): no fibre txs exist at app version 9.
    let mut normal: Vec<&[u8]> = Vec::new();
    let mut blobs: Vec<UnwrappedBlobTx> = Vec::new();
    for raw in within_bytes {
        if raw.len() > MAX_TX_SIZE {
            continue;
        }
        let b = txdecode::unmarshal_blob_tx_v9(raw);
        let sdk_bytes = match &b {
            Some(Ok(u)) => u.tx.as_slice(),
            _ => raw,
        };
        if txdecode::has_duplicate_tx_raw_field(sdk_bytes) {
            continue;
        }
        match b {
            Some(Ok(u)) => blobs.push(u),
            Some(Err(_)) => {}
            None => {
                if txdecode::decode_v9(raw).is_ok() {
                    normal.push(raw);
                }
            }
        }
    }

    let mut zeroed = false;
    let mut kept = Vec::new();
    let mut sdk_count = 0usize;
    for raw in &normal {
        let Ok(tx) = txdecode::decode_v9(raw) else {
            continue;
        };
        ctx.tx_bytes = raw.to_vec();
        let n = tx.msgs().len();
        if sdk_count + n > MAX_SDK_MESSAGES {
            continue;
        }
        if !builder.append_tx(raw) {
            continue;
        }
        if zeroed {
            let _ = builder.revert_last_tx();
            continue;
        }
        if let Err(e) = ante_fill(ctx, &tx, deps)? {
            zeroed |= e.zero_ctx;
            let _ = builder.revert_last_tx();
            continue;
        }
        sdk_count += n;
        kept.push(raw.to_vec());
    }

    let mut blob_msgs = 0usize;
    for u in &blobs {
        let Ok(tx) = txdecode::decode_v9(&u.tx) else {
            continue;
        };
        ctx.tx_bytes.clone_from(&u.tx);
        if blob_msgs + tx.msgs().len() > MAX_PFB_MESSAGES {
            continue;
        }
        let Ok(btx) = to_square_blob_tx(u) else {
            continue;
        };
        match builder.append_blob_tx(&btx) {
            Ok(true) => {}
            Ok(false) | Err(_) => continue,
        }
        if zeroed {
            let _ = builder.revert_last_blob_tx();
            continue;
        }
        if let Err(e) = ante_fill(ctx, &tx, deps)? {
            zeroed |= e.zero_ctx;
            let _ = builder.revert_last_blob_tx();
            continue;
        }
        blob_msgs += tx.msgs().len();
        // encodeBlobTxs: MarshalBlobTx of the decoded blob tx.
        kept.push(txdecode::marshal_blob_tx(&u.tx, &u.blobs));
    }
    Ok(kept)
}

/// celestia-app v9.0.8 `ProcessProposalHandler` (app version 9).
fn process_v9(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    req: &RequestProcessProposal,
    proposed: &ProposedSquare,
) -> Result<bool, Abort> {
    let (mut sdk_msgs, mut blob_msgs) = (0usize, 0usize);
    for raw in &req.txs {
        if raw.len() > MAX_TX_SIZE {
            return Ok(false);
        }
        let blob = match txdecode::unmarshal_blob_tx_v9(raw) {
            Some(Err(_)) => return Ok(false),
            Some(Ok(u)) => Some(u),
            None => None,
        };
        let sdk_bytes = blob.as_ref().map_or(raw.as_slice(), |u| u.tx.as_slice());
        ctx.tx_bytes = sdk_bytes.to_vec();
        let Ok(tx) = txdecode::decode_v9(sdk_bytes) else {
            return Ok(false);
        };
        let Some(blob) = blob else {
            if has_pfb(&tx) {
                return Ok(false);
            }
            sdk_msgs += tx.msgs().len();
            if sdk_msgs > MAX_SDK_MESSAGES {
                return Ok(false);
            }
            if ante(ctx, &tx, deps).is_err() {
                return Ok(false);
            }
            continue;
        };
        if validate_blob_tx(&tx, &blob).is_err() {
            return Ok(false);
        }
        blob_msgs += tx.msgs().len();
        if blob_msgs > MAX_PFB_MESSAGES {
            return Ok(false);
        }
        if ante(ctx, &tx, deps).is_err() {
            return Ok(false);
        }
    }
    // v9 reads the max square size after the ante loop, on the gas meter
    // the last ante handler left in the context; running out of gas there
    // panics and the proposal is rejected.
    let Ok(max_square_size) = max_effective_square_size(ctx) else {
        return Ok(false);
    };
    // rc4 `Construct(req.Txs)` classifies the raw txs itself.
    let Ok(classified) = classify_txs_v9(&req.txs) else {
        return Ok(false);
    };
    let Ok(square) = v4::construct_rc4(&classified, max_square_size, SUBTREE_ROOT_THRESHOLD) else {
        return Ok(false);
    };
    let Ok((width, root)) = proposed.data_root(&square) else {
        return Ok(false);
    };
    Ok(width as u64 == req.square_size && root == req.data_root_hash)
}

/// `processFibreTxsForSquare`.
fn process_fibre_txs(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    builder: &mut Builder,
    pff: &[&[u8]],
    zeroed: bool,
) -> Result<Vec<Vec<u8>>, Abort> {
    let mut kept = Vec::new();
    let mut fibre_msgs = 0usize;
    for raw in pff {
        let Ok(Some(fibre)) = fibre_tx::try_parse(raw) else {
            continue;
        };
        let Ok(tx) = txdecode::decode(raw) else {
            continue;
        };
        if fibre_msgs + tx.msgs().len() > MAX_PAY_FOR_FIBRE_MESSAGES {
            continue;
        }
        if zeroed {
            return Err(Abort("nil multistore".into()));
        }
        let base = ctx.multistore().depth();
        let cp = ctx.branch();
        ctx.tx_bytes = raw.to_vec();
        let discard = |ctx: &mut Context<'_>| {
            while ctx.multistore().depth() > base {
                ctx.multistore().discard_branch();
            }
        };
        if !matches!(builder.append_fibre_tx(&fibre), Ok(true)) {
            discard(ctx);
            continue;
        }
        let ok = match ante_fill(ctx, &tx, deps) {
            Ok(r) => r.is_ok(),
            Err(abort) => {
                discard(ctx);
                return Err(abort);
            }
        };
        if !ok || execute_tx_msgs(ctx, &tx).is_err() {
            let _ = builder.revert_last_pay_for_fibre_tx();
            discard(ctx);
            continue;
        }
        ctx.write(cp);
        fibre_msgs += tx.msgs().len();
        kept.push(raw.to_vec());
    }
    Ok(kept)
}

/// `ProcessProposalHandler`: `Ok(true)` accepts.
fn process(
    ctx: &mut Context<'_>,
    deps: &x_ante::StoreDeps,
    req: &RequestProcessProposal,
    proposed: &ProposedSquare,
) -> Result<bool, Abort> {
    if x_ante::is_v9(ctx) {
        return process_v9(ctx, deps, req, proposed);
    }
    let max_square_size = max_effective_square_size(ctx)?;
    if x_fibre::begin_block(ctx).is_err() {
        return Ok(false);
    }
    let (mut sdk_msgs, mut blob_msgs, mut fibre_msgs) = (0usize, 0usize, 0usize);
    for raw in &req.txs {
        if raw.len() > MAX_TX_SIZE {
            return Ok(false);
        }
        let blob = match txdecode::unmarshal_blob_tx(raw) {
            Some(Err(_)) => return Ok(false),
            Some(Ok(u)) => Some(u),
            None => None,
        };
        let sdk_bytes = blob.as_ref().map_or(raw.as_slice(), |u| u.tx.as_slice());
        ctx.tx_bytes = sdk_bytes.to_vec();
        let Ok(tx) = txdecode::decode(sdk_bytes) else {
            return Ok(false);
        };
        let Some(blob) = blob else {
            if has_pfb(&tx) || fibre_tx::validate_shape(&tx).is_err() {
                return Ok(false);
            }
            let is_pff = fibre_tx::is_pff(&tx);
            if is_pff {
                fibre_msgs += 1;
                if fibre_msgs > MAX_PAY_FOR_FIBRE_MESSAGES {
                    return Ok(false);
                }
            } else {
                sdk_msgs += count_executable_msgs(Some(&mut *ctx), tx.msgs())
                    .map_err(|_| Abort("nil multistore".into()))?;
                if sdk_msgs > MAX_SDK_MESSAGES {
                    return Ok(false);
                }
            }
            if ante(ctx, &tx, deps).is_err() {
                return Ok(false);
            }
            if is_pff {
                if execute_tx_msgs(ctx, &tx).is_err() {
                    return Ok(false);
                }
            } else if fibre_tx::contains_fibre_state_msg(&tx) {
                let _ = execute_tx_msgs(ctx, &tx);
            }
            continue;
        };
        if validate_blob_tx(&tx, &blob).is_err() {
            return Ok(false);
        }
        blob_msgs += tx.msgs().len();
        if blob_msgs > MAX_PFB_MESSAGES {
            return Ok(false);
        }
        if ante(ctx, &tx, deps).is_err() {
            return Ok(false);
        }
    }

    let Ok(classified) = classify_txs(&req.txs) else {
        return Ok(false);
    };
    let Ok(square) = v4::construct(&classified, max_square_size, SUBTREE_ROOT_THRESHOLD) else {
        return Ok(false);
    };
    let Ok((width, root)) = proposed.data_root(&square) else {
        return Ok(false);
    };
    Ok(width as u64 == req.square_size && root == req.data_root_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_block_data_root_matches_go() {
        // ProcessProposal.data_root_hash recorded from Go celestia-app v10.
        let (w, root) = data_root(&square::empty_square()).unwrap_or_default();
        assert_eq!(w, 1);
        assert_eq!(
            hex::encode(root),
            "3d96b7d238e7e0456f6af8e7cdf0a67bd6cf9c2089ecb559c659dcaa1f880353"
        );
    }

    /// A `MsgPayForBlobs` whose `blob_sizes` are packed 10-byte varints
    /// with high bits in the last byte: the tx decoder's unknown-field
    /// check does not parse packed runs and gogoproto keeps the low bits
    /// (`ValidateBlobTxSkipCommitment` reads `GetMsgs()`), so Go accepts
    /// the blob tx; prost rejects the varint. Also an element read past
    /// its packed run is not reachable without a valid field after it.
    #[test]
    fn blob_tx_validation_reads_the_gogoproto_message() {
        use celestia_proto::Any;
        use celestia_proto::cosmos::tx::v1beta1::TxBody;
        use prost::Message as _;
        let ns = shares::Namespace::new_v0(&[1u8; 10]).unwrap();
        let data = vec![7u8; 100];
        let blob = shares::Blob::new_v0(ns, data.clone()).unwrap();
        let c = square::create_commitment(&blob, merkle::root_from_byte_slices, 64).unwrap();
        let signer = encoding::bech32::encode("celestia", &[3u8; 20]).unwrap();
        let field = |num: u8, v: &[u8]| {
            let mut o = vec![num << 3 | 2, u8::try_from(v.len()).unwrap()];
            o.extend_from_slice(v);
            o
        };
        // 100 as 10 bytes, last byte 0x7e (bits past 64 that gogoproto drops).
        let mut long = vec![0xe4];
        long.extend([0x80; 8]);
        long.push(0x7e);
        let pfb = [
            field(1, signer.as_bytes()),
            field(2, ns.as_bytes()),
            field(3, &long),
            field(4, &c),
            field(8, &[0]),
        ]
        .concat();
        let msg = Any {
            type_url: "/celestia.blob.v1.MsgPayForBlobs".into(),
            value: pfb,
        };
        assert!(MsgPayForBlobs::decode(msg.value.as_slice()).is_err());
        let body = TxBody {
            messages: vec![msg],
            ..Default::default()
        };
        let raw = celestia_proto::cosmos::tx::v1beta1::TxRaw {
            body_bytes: body.encode_to_vec(),
            auth_info_bytes: Vec::new(),
            signatures: Vec::new(),
        }
        .encode_to_vec();
        let tx = txdecode::decode(&raw).unwrap();
        let u = UnwrappedBlobTx {
            tx: raw,
            blobs: vec![sdk::tx::Blob {
                namespace_id: ns.id().to_vec(),
                namespace_version: 0,
                share_version: 0,
                data,
                signer: Vec::new(),
            }],
        };
        validate_blob_tx(&tx, &u).unwrap();
    }

    #[test]
    fn proposed_square_is_reused_only_for_the_same_square() {
        let empty = square::empty_square();
        let other = square::Square::from_shares_unchecked(shares::tail_padding_shares(4));
        let fresh = |s: &square::Square| data_root(s).unwrap_or_default();
        // A cached entry with a deliberately wrong root shows which path ran.
        let cached = ProposedSquare(Some((empty.clone(), 1, vec![7])));
        assert_eq!(cached.data_root(&empty).unwrap_or_default(), (1, vec![7]));
        assert_eq!(cached.data_root(&other).unwrap_or_default(), fresh(&other));
        assert_eq!(
            ProposedSquare::default()
                .data_root(&empty)
                .unwrap_or_default(),
            fresh(&empty)
        );
    }
}
