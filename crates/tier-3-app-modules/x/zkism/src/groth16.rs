//! SP1 Groth16 (BN254) proof verification as celestia-app v10 x/zkism does
//! it with gnark v0.16.3 / gnark-crypto v0.21.0: `types/verifier.go`,
//! `internal/groth16`, gnark's `VerifyingKey.ReadFrom` (point decoding with
//! subgroup checks) and `groth16.Verify`.

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine};
use ark_ec::CurveGroup;
use ark_ec::pairing::Pairing;
use ark_ec::short_weierstrass::SWCurveConfig;
use ark_ff::{BigInt, Field, PrimeField, Zero};

/// `PrefixLen`.
pub const PREFIX_LEN: usize = 4;
/// `MetadataLen`: exit code, vk root and proof nonce (32 bytes each).
pub const METADATA_LEN: usize = 96;
/// `ProofSize`: uncompressed `Ar`, `Bs`, `Krs`.
pub const PROOF_SIZE: usize = 256;
/// `ProofBytesLen`.
pub const PROOF_BYTES_LEN: usize = PREFIX_LEN + METADATA_LEN + PROOF_SIZE;

/// SP1 `vk_root` the proofs must commit to.
pub const VK_ROOT: [u8; 32] = [
    0x00, 0x2f, 0x85, 0x0e, 0xe9, 0x98, 0x97, 0x4d, 0x6c, 0xc0, 0x0e, 0x50, 0xcd, 0x08, 0x14, 0xb0,
    0x98, 0xc0, 0x5b, 0xfa, 0xde, 0x46, 0x6d, 0x28, 0x57, 0x32, 0x40, 0xd0, 0x57, 0xf2, 0x53, 0x52,
];

const M_MASK: u8 = 0b11 << 6;
const M_UNCOMPRESSED: u8 = 0;
const M_COMPRESSED_SMALLEST: u8 = 0b10 << 6;
const M_COMPRESSED_LARGEST: u8 = 0b11 << 6;
const M_COMPRESSED_INFINITY: u8 = 0b01 << 6;

/// A decoding / verification failure (the message is for logs only).
pub type Error = String;

/// gnark `fp.Element.SetBytesCanonical` on 32 big-endian bytes.
fn fq_canonical(b: &[u8]) -> Result<Fq, Error> {
    let mut limbs = [0u64; 4];
    for (i, chunk) in b.chunks(8).enumerate() {
        let mut w = [0u8; 8];
        w.copy_from_slice(chunk);
        limbs[3 - i] = u64::from_be_bytes(w);
    }
    Fq::from_bigint(BigInt(limbs)).ok_or_else(|| "invalid fp.Element encoding".to_owned())
}

/// gnark `fp.Element.LexicographicallyLargest`: `z > (q-1)/2`.
fn fq_largest(z: &Fq) -> bool {
    z.into_bigint() > Fq::MODULUS_MINUS_ONE_DIV_TWO
}

/// gnark `E2.LexicographicallyLargest`.
fn fq2_largest(z: &Fq2) -> bool {
    if z.c1.is_zero() {
        fq_largest(&z.c0)
    } else {
        fq_largest(&z.c1)
    }
}

/// Reads `n` bytes.
fn take<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if buf.len() < n {
        return Err("unexpected EOF".into());
    }
    let (a, b) = buf.split_at(n);
    *buf = b;
    Ok(a)
}

fn is_zeroed(first: u8, rest: &[u8]) -> bool {
    first == 0 && rest.iter().all(|b| *b == 0)
}

/// gnark `Decoder.Decode(*G1Affine)` with subgroup checks.
#[allow(clippy::many_single_char_names)]
pub fn decode_g1(buf: &mut &[u8]) -> Result<G1Affine, Error> {
    let head = *buf.first().ok_or("unexpected EOF")?;
    let m = head & M_MASK;
    if m == M_UNCOMPRESSED {
        let b = take(buf, 64)?;
        let x = fq_canonical(&b[..32])?;
        let y = fq_canonical(&b[32..])?;
        if x.is_zero() && y.is_zero() {
            return Ok(G1Affine::identity());
        }
        let p = G1Affine::new_unchecked(x, y);
        if !p.is_on_curve() {
            return Err("invalid point: subgroup check failed".into());
        }
        return Ok(p);
    }
    let b = take(buf, 32)?;
    if m == M_COMPRESSED_INFINITY {
        if !is_zeroed(b[0] & !M_MASK, &b[1..]) {
            return Err("invalid infinity point encoding".into());
        }
        return Ok(G1Affine::identity());
    }
    let mut xb = [0u8; 32];
    xb.copy_from_slice(b);
    xb[0] &= !M_MASK;
    let x = fq_canonical(&xb)?;
    let y2 = x.square() * x + ark_bn254::g1::Config::COEFF_B;
    let mut y = y2
        .sqrt()
        .ok_or("invalid compressed coordinate: square root doesn't exist")?;
    let largest = fq_largest(&y);
    if (largest && m == M_COMPRESSED_SMALLEST) || (!largest && m == M_COMPRESSED_LARGEST) {
        y = -y;
    }
    let p = G1Affine::new_unchecked(x, y);
    if !p.is_on_curve() {
        return Err("invalid point: subgroup check failed".into());
    }
    Ok(p)
}

fn g2_checked(p: G2Affine) -> Result<G2Affine, Error> {
    if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
        return Err("invalid point: subgroup check failed".into());
    }
    Ok(p)
}

/// gnark `Decoder.Decode(*G2Affine)` with subgroup checks (`x = A1 || A0`).
#[allow(clippy::many_single_char_names)]
pub fn decode_g2(buf: &mut &[u8]) -> Result<G2Affine, Error> {
    let head = *buf.first().ok_or("unexpected EOF")?;
    let m = head & M_MASK;
    if m == M_UNCOMPRESSED {
        let b = take(buf, 128)?;
        let x = Fq2::new(fq_canonical(&b[32..64])?, fq_canonical(&b[..32])?);
        let y = Fq2::new(fq_canonical(&b[96..128])?, fq_canonical(&b[64..96])?);
        if x.is_zero() && y.is_zero() {
            return Ok(G2Affine::identity());
        }
        return g2_checked(G2Affine::new_unchecked(x, y));
    }
    let b = take(buf, 64)?;
    if m == M_COMPRESSED_INFINITY {
        if !is_zeroed(b[0] & !M_MASK, &b[1..]) {
            return Err("invalid infinity point encoding".into());
        }
        return Ok(G2Affine::identity());
    }
    let mut a1 = [0u8; 32];
    a1.copy_from_slice(&b[..32]);
    a1[0] &= !M_MASK;
    let x = Fq2::new(fq_canonical(&b[32..64])?, fq_canonical(&a1)?);
    let y2 = x.square() * x + ark_bn254::g2::Config::COEFF_B;
    if y2.legendre().is_qnr() {
        return Err("invalid compressed coordinate: square root doesn't exist".into());
    }
    let mut y = y2
        .sqrt()
        .ok_or("invalid compressed coordinate: square root doesn't exist")?;
    let largest = fq2_largest(&y);
    if (largest && m == M_COMPRESSED_SMALLEST) || (!largest && m == M_COMPRESSED_LARGEST) {
        y = -y;
    }
    g2_checked(G2Affine::new_unchecked(x, y))
}

fn read_u32(buf: &mut &[u8]) -> Result<u32, Error> {
    let b = take(buf, 4)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A gnark BN254 Groth16 verifying key (without commitment keys).
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyingKey {
    /// `[α]1`.
    pub alpha: G1Affine,
    /// `[β]2`.
    pub beta: G2Affine,
    /// `[γ]2`.
    pub gamma: G2Affine,
    /// `[δ]2`.
    pub delta: G2Affine,
    /// `[Kvk]1`.
    pub k: Vec<G1Affine>,
    /// `len(PublicAndCommitmentCommitted)`.
    pub public_committed: usize,
}

/// `groth16.NewVerifyingKey(bytes)`: `VerifyingKey.ReadFrom` must consume
/// exactly `bz`.
pub fn new_verifying_key(bz: &[u8]) -> Result<VerifyingKey, Error> {
    let mut buf = bz;
    let r = &mut buf;
    let alpha = decode_g1(r)?;
    let _g1_beta = decode_g1(r)?;
    let beta = decode_g2(r)?;
    let gamma = decode_g2(r)?;
    let _g1_delta = decode_g1(r)?;
    let delta = decode_g2(r)?;
    let nk = read_u32(r)?;
    let mut k = Vec::new();
    for _ in 0..nk {
        k.push(decode_g1(r)?);
    }
    // [][]uint64: outer length, then each inner slice (length + u64s).
    let np = read_u32(r)?;
    for _ in 0..np {
        let n = read_u32(r)?;
        take(r, 8 * n as usize)?;
    }
    let nb_commitments = read_u32(r)?;
    if nb_commitments != 0 {
        return Err("commitment keys are not supported".into());
    }
    if !buf.is_empty() {
        return Err(format!(
            "invalid key length: expected {}, got {}",
            bz.len(),
            bz.len() - buf.len()
        ));
    }
    Ok(VerifyingKey {
        alpha,
        beta,
        gamma,
        delta,
        k,
        public_committed: np as usize,
    })
}

/// A Groth16 proof.
#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    /// `Ar`.
    pub ar: G1Affine,
    /// `Bs`.
    pub bs: G2Affine,
    /// `Krs`.
    pub krs: G1Affine,
}

/// `groth16.UnmarshalProof`.
pub fn unmarshal_proof(bz: &[u8]) -> Result<Proof, Error> {
    let mut buf = bz;
    let r = &mut buf;
    let wrap = |e: Error| format!("error unmarshaling proof: {e}");
    Ok(Proof {
        ar: decode_g1(r).map_err(wrap)?,
        bs: decode_g2(r).map_err(wrap)?,
        krs: decode_g1(r).map_err(wrap)?,
    })
}

/// `groth16.Verify` for a key without commitments.
pub fn verify(proof: &Proof, vk: &VerifyingKey, public: &[Fr]) -> Result<(), Error> {
    let nb_public = vk.k.len() - vk.public_committed.min(vk.k.len());
    if public.len() + 1 != nb_public {
        return Err(format!(
            "invalid witness size, got {}, expected {} (public - ONE_WIRE)",
            public.len(),
            nb_public.saturating_sub(1)
        ));
    }
    let mut k_sum = G1Projective::from(vk.k[0]);
    for (w, k) in public.iter().zip(&vk.k[1..]) {
        k_sum += *k * w;
    }
    let k_sum = k_sum.into_affine();
    let out = Bn254::multi_pairing(
        [proof.ar, k_sum, proof.krs, -vk.alpha],
        [proof.bs, -vk.gamma, -vk.delta, vk.beta],
    );
    if out.is_zero() {
        Ok(())
    } else {
        Err("pairing doesn't match".into())
    }
}

/// `HashBN254`: sha256 with the top three bits cleared.
#[must_use]
pub fn hash_bn254(data: &[u8]) -> [u8; 32] {
    let mut h = hash::sha256(data);
    h[0] &= 0b0001_1111;
    h
}

/// `NewSP1Groth16Verifier` + `VerifyProof` errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// `ErrInvalidProofLength`.
    Length(String),
    /// `ErrInvalidProofPrefix`.
    Prefix(String),
    /// `ErrInvalidProof`.
    Invalid(String),
    /// Proof point decoding (unregistered error).
    Decode(String),
}

/// celestia-app v9.0.8 `SP1Groth16Verifier.VerifyProof` (SP1 v5, app
/// version 9): `prefix(4) | proof(256)` and two public inputs
/// `[vkey_hash, committed_values_digest]`.
pub fn sp1_verify_v5(
    vk: &VerifyingKey,
    vk_bytes: &[u8],
    proof_bz: &[u8],
    program_vk: &[u8],
    public_values: &[u8],
) -> Result<(), VerifyError> {
    let want = PREFIX_LEN + PROOF_SIZE;
    if proof_bz.len() != want {
        return Err(VerifyError::Length(format!(
            "expected {want}, got {}",
            proof_bz.len()
        )));
    }
    let prefix = &hash::sha256(vk_bytes)[..PREFIX_LEN];
    if prefix != &proof_bz[..PREFIX_LEN] {
        return Err(VerifyError::Prefix(format!(
            "expected {}, got {}",
            hex::encode(prefix),
            hex::encode(&proof_bz[..PREFIX_LEN])
        )));
    }
    let proof = unmarshal_proof(&proof_bz[PREFIX_LEN..])
        .map_err(|e| VerifyError::Decode(format!("failed to unmarshal proof: {e}")))?;
    let fr = Fr::from_be_bytes_mod_order;
    let public = [fr(program_vk), fr(&hash_bn254(public_values))];
    verify(&proof, vk, &public)
        .map_err(|e| VerifyError::Invalid(format!("failed to verify proof: {e}")))
}

/// `SP1Groth16Verifier.VerifyProof` with the verifier built from `vk_bytes`
/// (the vkey was validated when the ISM was created).
pub fn sp1_verify(
    vk: &VerifyingKey,
    vk_bytes: &[u8],
    proof_bz: &[u8],
    program_vk: &[u8],
    public_values: &[u8],
) -> Result<(), VerifyError> {
    if proof_bz.len() != PROOF_BYTES_LEN {
        return Err(VerifyError::Length(format!(
            "expected {PROOF_BYTES_LEN}, got {}",
            proof_bz.len()
        )));
    }
    let prefix = &hash::sha256(vk_bytes)[..PREFIX_LEN];
    if prefix != &proof_bz[..PREFIX_LEN] {
        return Err(VerifyError::Prefix(format!(
            "expected {}, got {}",
            hex::encode(prefix),
            hex::encode(&proof_bz[..PREFIX_LEN])
        )));
    }
    let exit_code = &proof_bz[PREFIX_LEN..PREFIX_LEN + 32];
    let vk_root = &proof_bz[PREFIX_LEN + 32..PREFIX_LEN + 64];
    let nonce = &proof_bz[PREFIX_LEN + 64..PREFIX_LEN + 96];
    if exit_code.iter().any(|b| *b != 0) {
        return Err(VerifyError::Invalid(format!(
            "non-zero exit code: {}",
            hex::encode(exit_code)
        )));
    }
    if vk_root != VK_ROOT {
        return Err(VerifyError::Invalid(format!(
            "vk_root mismatch: expected {}, got {}",
            hex::encode(VK_ROOT),
            hex::encode(vk_root)
        )));
    }
    let proof = unmarshal_proof(&proof_bz[PREFIX_LEN + METADATA_LEN..])
        .map_err(|e| VerifyError::Decode(format!("failed to unmarshal proof: {e}")))?;
    let fr = Fr::from_be_bytes_mod_order;
    let public = [
        fr(program_vk),
        fr(&hash_bn254(public_values)),
        fr(exit_code),
        fr(vk_root),
        fr(nonce),
    ];
    verify(&proof, vk, &public)
        .map_err(|e| VerifyError::Invalid(format!("failed to verify proof: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK: &[u8] = include_bytes!("../testdata/groth16_vk.bin");
    const ST_PROOF: &[u8] = include_bytes!("../testdata/state_transition/proof.bin");
    const ST_VALUES: &[u8] = include_bytes!("../testdata/state_transition/public_values.bin");
    const SM_PROOF: &[u8] = include_bytes!("../testdata/state_membership/proof.bin");
    const SM_VALUES: &[u8] = include_bytes!("../testdata/state_membership/public_values.bin");

    // Program vkey commitments of the celestia-app keeper tests.
    const ST_VKEY: &str = "0x004ac29c473e811dece0f8dd76c8eda80f886d263efb393ec81f54173e54f160";
    const SM_VKEY: &str = "0x00982fb21526d096c8bf58eda36b5e293ee9ea0f36df441f6a996a974f8feb63";

    fn vkey(s: &str) -> Vec<u8> {
        hex::decode(s.trim_start_matches("0x")).unwrap_or_default()
    }

    #[test]
    fn decodes_go_testdata_vk() {
        let vk = new_verifying_key(VK);
        assert!(vk.is_ok(), "{vk:?}");
        assert_eq!(vk.map_or(0, |v| v.k.len()), 6);
        assert!(new_verifying_key(&VK[..VK.len() - 1]).is_err());
        let mut longer = VK.to_vec();
        longer.push(0);
        assert!(new_verifying_key(&longer).is_err());
    }

    #[test]
    fn verifies_go_testdata_proofs() -> Result<(), String> {
        let vk = new_verifying_key(VK)?;
        let st = sp1_verify(&vk, VK, ST_PROOF, &vkey(ST_VKEY), ST_VALUES);
        let sm = sp1_verify(&vk, VK, SM_PROOF, &vkey(SM_VKEY), SM_VALUES);
        assert_eq!(st, Ok(()));
        assert_eq!(sm, Ok(()));
        // Wrong program, tampered values, tampered proof.
        assert!(sp1_verify(&vk, VK, ST_PROOF, &vkey(SM_VKEY), ST_VALUES).is_err());
        let mut v = ST_VALUES.to_vec();
        v[20] ^= 1;
        assert!(sp1_verify(&vk, VK, ST_PROOF, &vkey(ST_VKEY), &v).is_err());
        let mut p = ST_PROOF.to_vec();
        p[200] ^= 1;
        assert!(sp1_verify(&vk, VK, &p, &vkey(ST_VKEY), ST_VALUES).is_err());
        Ok(())
    }

    /// celestia-app v9.0.8's SP1 v5 vectors (`x/zkism/internal/testdata`,
    /// program vkeys from its keeper tests).
    #[test]
    fn verifies_v9_sp1_v5_proofs() -> Result<(), String> {
        const VK9: &[u8] = include_bytes!("../testdata/v9/groth16_vk.bin");
        const ST9: &[u8] = include_bytes!("../testdata/v9/state_transition/proof.bin");
        const ST9_VALUES: &[u8] =
            include_bytes!("../testdata/v9/state_transition/public_values.bin");
        const SM9: &[u8] = include_bytes!("../testdata/v9/state_membership/proof.bin");
        const SM9_VALUES: &[u8] =
            include_bytes!("../testdata/v9/state_membership/public_values.bin");
        let transition_vkey =
            vkey("0x0017bc91d53b93c46eb842d7f9020a94ea13d8877a21608b34b71fcc4da64f29");
        let membership_vkey =
            vkey("0x004959d5fb2c3d5bc1f98e032188dd94fbb5c6b6152df356c7c20be23be824a2");
        let vk = new_verifying_key(VK9)?;
        assert_eq!(vk.k.len(), 3);
        assert_eq!(
            sp1_verify_v5(&vk, VK9, ST9, &transition_vkey, ST9_VALUES),
            Ok(())
        );
        assert_eq!(
            sp1_verify_v5(&vk, VK9, SM9, &membership_vkey, SM9_VALUES),
            Ok(())
        );
        assert!(sp1_verify_v5(&vk, VK9, ST9, &membership_vkey, ST9_VALUES).is_err());
        // A v5 proof is the wrong length for the v10 (SP1 v6) verifier.
        assert!(matches!(
            sp1_verify(&vk, VK9, ST9, &transition_vkey, ST9_VALUES),
            Err(VerifyError::Length(_))
        ));
        Ok(())
    }
}
