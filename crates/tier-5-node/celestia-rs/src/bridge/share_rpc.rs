//! celestia-node share responses over a consensus-authenticated square.
use anyhow::{Result, ensure};
use rpc::json::J;
use square::{DataAvailabilityHeader, ExtendedDataSquare};

pub(super) enum Read {
    Available,
    Eds,
    Row(usize),
    Samples(Vec<(usize, usize)>),
    Range(usize, usize),
}

impl Read {
    pub(super) fn response(
        self,
        eds: &ExtendedDataSquare,
        dah: &DataAvailabilityHeader,
    ) -> Result<J> {
        match self {
            Self::Available => Ok(J::Null),
            Self::Eds => {
                let width = eds.width();
                let mut shares = Vec::with_capacity(width * width);
                for row in 0..width {
                    for col in 0..width {
                        shares.push(J::b64(eds.share(row, col)?));
                    }
                }
                Ok(J::obj([
                    ("data_square", J::Arr(shares)),
                    ("codec", J::str("Leopard")),
                ]))
            }
            Self::Row(index) => {
                let row = eds.row(index)?;
                Ok(J::obj([
                    (
                        "shares",
                        J::Arr(
                            row[..eds.original_width()]
                                .iter()
                                .map(|s| J::b64(s))
                                .collect(),
                        ),
                    ),
                    ("side", J::str("LEFT")),
                ]))
            }
            Self::Samples(coords) => {
                let mut samples = Vec::with_capacity(coords.len());
                for (row, col) in coords {
                    let cell = eds.share(row, col)?;
                    let proof = app::inclusion_proof::row_range_proof(eds, dah, row, col, col + 1)
                        .map_err(|e| anyhow::anyhow!("{}", e.log))?;
                    samples.push(J::obj([
                        ("share", J::b64(cell)),
                        ("proof", super::proof_json(&proof, true)),
                        ("proof_type", J::num(0)),
                    ]));
                }
                Ok(J::Arr(samples))
            }
            Self::Range(start, end) => range(eds, dah, start, end, None),
        }
    }
}

pub(super) fn range(
    eds: &ExtendedDataSquare,
    dah: &DataAvailabilityHeader,
    start: usize,
    end: usize,
    subtree_roots: Option<Vec<Vec<u8>>>,
) -> Result<J> {
    let width = eds.original_width();
    ensure!(
        start < end && end <= width * width,
        "invalid original share range"
    );
    let first = start / width;
    let last = (end - 1) / width;
    let ns =
        shares::Namespace::from_slice(&eds.share(first, start % width)?[..shares::NAMESPACE_SIZE])?;
    let mut data = Vec::with_capacity(end - start);
    let mut proofs = Vec::new();
    for row in first..=last {
        let from = start.saturating_sub(row * width);
        let stop = (end - row * width).min(width);
        for col in from..stop {
            let share = eds.share(row, col)?;
            ensure!(
                &share[..shares::NAMESPACE_SIZE] == ns.as_bytes(),
                "range spans namespaces"
            );
            if subtree_roots.is_none() {
                data.push(J::b64(share));
            }
        }
        let proof = app::inclusion_proof::row_range_proof(eds, dah, row, from, stop)
            .map_err(|e| anyhow::anyhow!("{}", e.log))?;
        proofs.push(super::proof_json(&proof, subtree_roots.is_some()));
    }
    let row_proof = row_proof_json(dah, first, last, subtree_roots.is_some());
    if let Some(roots) = subtree_roots {
        return Ok(J::obj([
            (
                "subtree_roots",
                J::Arr(roots.iter().map(|r| J::b64(r)).collect()),
            ),
            ("subtree_root_proofs", J::Arr(proofs)),
            ("namespace_id", J::b64(&ns.id())),
            ("namespace_version", J::num(ns.version())),
            ("row_proof", row_proof),
        ]));
    }
    Ok(range_result(ns, data, proofs, row_proof))
}

pub(super) fn range_result(ns: shares::Namespace, data: Vec<J>, proofs: Vec<J>, row_proof: J) -> J {
    let proof = J::obj([
        ("data", J::Arr(data.clone())),
        ("share_proofs", J::Arr(proofs)),
        ("namespace_id", J::b64(&ns.id())),
        ("namespace_version", J::num(ns.version())),
        ("row_proof", row_proof),
    ]);
    J::obj([("Shares", J::Arr(data)), ("Proof", proof)])
}

pub(super) fn row_proof_json(
    dah: &DataAvailabilityHeader,
    first: usize,
    last: usize,
    base64_roots: bool,
) -> J {
    let roots: Vec<_> = dah.row_roots.iter().chain(&dah.column_roots).collect();
    let (_, paths) = merkle::proofs_from_byte_slices(&roots);
    let row_paths = paths[first..=last]
        .iter()
        .map(|path| {
            J::obj([
                ("total", J::u64s(path.total)),
                ("index", J::u64s(path.index)),
                ("leaf_hash", J::b64(&path.leaf_hash)),
                (
                    "aunts",
                    J::Arr(path.aunts.iter().map(|n| J::b64(n)).collect()),
                ),
            ])
        })
        .collect();
    J::obj([
        (
            "row_roots",
            J::Arr(
                dah.row_roots[first..=last]
                    .iter()
                    .map(|r| if base64_roots { J::b64(r) } else { J::hex(r) })
                    .collect(),
            ),
        ),
        ("proofs", J::Arr(row_paths)),
        ("start_row", J::num(first)),
        ("end_row", J::num(last)),
    ])
}
