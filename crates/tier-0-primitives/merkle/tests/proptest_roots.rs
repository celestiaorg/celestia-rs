//! Property tests for the merkle crate.

#![allow(clippy::expect_used)]

use merkle::{proofs_from_byte_slices, root_from_byte_slices};
use proptest::prelude::*;

proptest! {
    #[test]
    fn generated_proofs_verify(items in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..128), 0..64)) {
        let (root, proofs) = proofs_from_byte_slices(&items);
        prop_assert_eq!(root, root_from_byte_slices(&items));
        prop_assert_eq!(proofs.len(), items.len());

        for (index, proof) in proofs.iter().enumerate() {
            prop_assert_eq!(usize::try_from(proof.index).expect("index fits usize"), index);
            prop_assert!(proof.verify(&root, &items[index]).is_ok());
        }
    }
}
