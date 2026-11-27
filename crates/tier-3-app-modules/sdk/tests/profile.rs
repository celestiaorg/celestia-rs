//! The phase profile records spans when enabled: time, committed IAVL
//! reads and node-cache lookups per phase. (Its own test binary: the
//! profile is process-wide.)

use sdk::store::{Kind, MultiStore, StoreError};

#[test]
fn spans_count_committed_reads() -> Result<(), StoreError> {
    sdk::profile::set_enabled(true);
    assert!(sdk::profile::enabled());
    let mut m = MultiStore::new();
    m.mount("a", Kind::Iavl)?;
    m.load_latest_version()?;
    m.set("a", b"k".to_vec(), b"v".to_vec())?;
    m.flush()?;
    m.commit()?;
    {
        let _span = sdk::profile::span("reads");
        for _ in 0..3 {
            assert_eq!(m.get("a", b"k")?, Some(b"v".to_vec()));
        }
    }
    drop(sdk::profile::span("empty"));
    let totals: std::collections::BTreeMap<_, _> = sdk::profile::take().into_iter().collect();
    assert_eq!(totals["reads"].count, 1);
    assert_eq!(totals["reads"].reads, 3);
    assert_eq!(totals["empty"].reads, 0);
    assert!(sdk::profile::take().is_empty(), "take resets");
    Ok(())
}
