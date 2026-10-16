//! `FileWal` group tests: ports of celestia-core v0.42.3
//! `libs/autofile/group_test.go` and `consensus/wal_test.go`, rotation,
//! pruning, search across files, the upgrade from head-only WALs, repair,
//! and the Go cross-compatibility vectors
//! (`conformance/consensusvectors/wal` → `conformance/fixtures/consensus/wal.json`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ctypes::Time;

use super::*;
use crate::types::{RoundStep, TimeoutInfo};

fn t(ms: i64) -> Time {
    Time {
        seconds: 1_700_000_000 + ms / 1000,
        nanos: i32::try_from((ms % 1000) * 1_000_000).unwrap(),
    }
}

fn end(h: i64) -> TimedWalMessage {
    TimedWalMessage {
        time: t(h),
        msg: WalMessage::EndHeight(h),
    }
}

/// The records of one height (without its `#ENDHEIGHT`).
fn height_records(h: i64) -> Vec<TimedWalMessage> {
    let mut v = vec![TimedWalMessage {
        time: t(h),
        msg: WalMessage::RoundState {
            height: h,
            round: 0,
            step: "RoundStepNewHeight".into(),
        },
    }];
    for r in 0..4 {
        v.push(TimedWalMessage {
            time: t(h * 10 + r),
            msg: WalMessage::Timeout(TimeoutInfo {
                duration: 1_000_000_000,
                height: h,
                round: i32::try_from(r).unwrap(),
                step: RoundStep::Propose,
            }),
        });
    }
    v
}

/// Manual checks only (the tests drive rotation and pruning).
fn manual(head: u64, total: u64) -> GroupOptions {
    GroupOptions {
        head_size_limit: head,
        total_size_limit: total,
        group_check_duration: Duration::ZERO,
        flush_interval: Duration::ZERO,
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn info(min: usize, max: usize, total: u64, head: u64) -> GroupInfo {
    GroupInfo {
        min_index: min,
        max_index: max,
        total_size: total,
        head_size: head,
    }
}

/// Writes heights `1..=n` like consensus does (#ENDHEIGHT with WriteSync)
/// and runs the head check after each height, returning the whole stream.
fn write_heights(wal: &mut FileWal, from: i64, to: i64) -> Vec<TimedWalMessage> {
    let mut all = Vec::new();
    for h in from..=to {
        for m in height_records(h) {
            wal.write(&m).unwrap();
            all.push(m);
        }
        wal.write_sync(&end(h)).unwrap();
        all.push(end(h));
        wal.check_head_size_limit().unwrap();
    }
    all
}

/// The records after the (unique) `#ENDHEIGHT h` in a flat stream.
fn after(stream: &[TimedWalMessage], h: i64) -> Option<Vec<TimedWalMessage>> {
    let i = stream
        .iter()
        .position(|m| m.msg == WalMessage::EndHeight(h))?;
    Some(stream[i + 1..].to_vec())
}

// ---------------------------------------------------------------------------
// autofile.Group (group_test.go)

/// `TestCheckHeadSizeLimit`.
#[test]
fn group_check_head_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Group::open(dir.path().join("wal"), &manual(1000 * 1000, 0)).unwrap();
    let line = |i: usize| {
        let mut l = format!("{i:0999}").into_bytes();
        l.push(b'\n');
        l
    };
    assert_eq!(g.read_group_info().unwrap(), info(0, 0, 0, 0));
    for i in 0..999 {
        g.write(&line(i)).unwrap();
    }
    g.flush_and_sync().unwrap();
    assert_eq!(g.read_group_info().unwrap(), info(0, 0, 999_000, 999_000));
    // Even calling checkHeadSizeLimit manually won't rotate it.
    g.check_head_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(0, 0, 999_000, 999_000));
    g.write(&line(999)).unwrap();
    g.flush_and_sync().unwrap();
    // Calling checkHeadSizeLimit this time rolls it.
    g.check_head_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(0, 1, 1_000_000, 0));
    g.write(&line(1000)).unwrap();
    g.flush_and_sync().unwrap();
    g.check_head_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(0, 1, 1_001_000, 1000));
    for i in 0..999 {
        g.write(&line(i)).unwrap();
    }
    g.flush_and_sync().unwrap();
    assert_eq!(
        g.read_group_info().unwrap(),
        info(0, 1, 2_000_000, 1_000_000)
    );
    // Calling checkHeadSizeLimit rolls it again.
    g.check_head_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(0, 2, 2_000_000, 0));
    // The head is recreated by the next write (or size check).
    assert_eq!(names(dir.path()), ["wal.000", "wal.001"]);
}

/// `TestRotateFile` + `TestMinIndex` / `TestMaxIndex`.
#[test]
fn group_rotate_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Group::open(dir.path().join("wal"), &manual(0, 0)).unwrap();
    assert_eq!((g.min_index, g.max_index), (0, 0));
    g.write(b"Line 1\nLine 2\nLine 3\n").unwrap();
    g.flush_and_sync().unwrap();
    g.rotate_file().unwrap();
    assert_eq!((g.min_index, g.max_index), (0, 1));
    g.write(b"Line 4\nLine 5\nLine 6\n").unwrap();
    g.flush_and_sync().unwrap();
    assert_eq!(
        fs::read(dir.path().join("wal.000")).unwrap(),
        b"Line 1\nLine 2\nLine 3\n"
    );
    assert_eq!(
        fs::read(dir.path().join("wal")).unwrap(),
        b"Line 4\nLine 5\nLine 6\n"
    );
    // Reopening finds the rotated file.
    drop(g);
    let g = Group::open(dir.path().join("wal"), &manual(0, 0)).unwrap();
    assert_eq!((g.min_index, g.max_index), (0, 1));
}

/// `TestGroupReaderRead` / `TestGroupReaderRead2`: one stream across files.
#[test]
fn group_reader_reads_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Group::open(dir.path().join("wal"), &manual(0, 0)).unwrap();
    g.write(b"Professor Monster").unwrap();
    g.flush_and_sync().unwrap();
    g.rotate_file().unwrap();
    g.write(b"Frankenstein").unwrap();
    g.flush_and_sync().unwrap();
    let srcs = g.sources();
    let mut rd = GroupReader::new(&srcs);
    let mut buf = vec![0u8; "Professor MonsterFrankenstein's Monster".len()];
    let n = rd.read_full(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"Professor MonsterFrankenstein");
    assert_eq!(rd.read_full(&mut [0u8; 1]).unwrap(), 0);
    // From the head only.
    let mut rd = GroupReader::new(&srcs[1..]);
    let mut buf = [0u8; 12];
    assert_eq!(rd.read_full(&mut buf).unwrap(), 12);
    assert_eq!(&buf, b"Frankenstein");
}

/// `bufio.Writer` semantics: a 40 KiB buffer that spills to the file.
#[test]
fn group_buffer_spills_like_bufio() {
    let dir = tempfile::tempdir().unwrap();
    let mut g = Group::open(dir.path().join("wal"), &manual(0, 0)).unwrap();
    g.write(&vec![1u8; 30_000]).unwrap();
    assert_eq!(g.head_size().unwrap(), 0);
    g.write(&vec![2u8; 30_000]).unwrap();
    // The buffer was filled to 40960 bytes and flushed.
    assert_eq!(g.head_size().unwrap(), 40_960);
    assert_eq!(g.buf.len(), 60_000 - 40_960);
    // A write larger than the buffer goes straight through when it is empty.
    g.flush().unwrap();
    g.write(&vec![3u8; 50_000]).unwrap();
    assert_eq!(g.head_size().unwrap(), 110_000);
    assert!(g.buf.is_empty());
}

/// `checkTotalSizeLimit`: up to four of the oldest files per check, never
/// the head; non-indexed files with the head's prefix count towards the
/// total.
#[test]
fn group_total_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10 {
        fs::write(dir.path().join(format!("wal.{i:03}")), [0u8; 100]).unwrap();
    }
    fs::write(dir.path().join("wal"), [0u8; 50]).unwrap();
    fs::write(dir.path().join("wal.CORRUPTED"), [0u8; 20]).unwrap();
    fs::write(dir.path().join("other"), [0u8; 1000]).unwrap();
    let mut g = Group::open(dir.path().join("wal"), &manual(0, 500)).unwrap();
    assert_eq!(g.read_group_info().unwrap(), info(0, 10, 1070, 50));
    g.check_total_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(4, 10, 670, 50));
    g.check_total_size_limit();
    assert_eq!(g.read_group_info().unwrap(), info(6, 10, 470, 50));
    g.check_total_size_limit();
    assert_eq!(
        names(dir.path()),
        [
            "other",
            "wal",
            "wal.006",
            "wal.007",
            "wal.008",
            "wal.009",
            "wal.CORRUPTED"
        ]
    );
    // Degenerate case: the head alone is over the limit; it is kept.
    let mut g = Group::open(dir.path().join("wal"), &manual(0, 10)).unwrap();
    for _ in 0..3 {
        g.check_total_size_limit();
    }
    assert_eq!(names(dir.path()), ["other", "wal", "wal.CORRUPTED"]);
    assert_eq!(g.read_group_info().unwrap(), info(0, 0, 70, 50));
}

#[test]
fn indexed_file_names() {
    assert_eq!(indexed_file("wal.000"), Some(0));
    assert_eq!(indexed_file("wal.123"), Some(123));
    assert_eq!(indexed_file("wal.1000"), Some(1000));
    assert_eq!(indexed_file("wal.01"), None);
    assert_eq!(indexed_file("wal.CORRUPTED"), None);
    assert_eq!(indexed_file("wal.tmp"), None);
    assert_eq!(indexed_file(".000"), None);
    assert_eq!(
        file_path_for_index(Path::new("/d/wal"), 7, 9),
        Path::new("/d/wal.007")
    );
    assert_eq!(
        file_path_for_index(Path::new("/d/wal"), 9, 9),
        Path::new("/d/wal")
    );
}

// ---------------------------------------------------------------------------
// BaseWAL over the group

#[test]
fn file_wal_rotates_and_searches_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cs.wal").join("wal");
    let mut wal = FileWal::open_with(&path, t(0), manual(1500, 0)).unwrap();
    let mut stream = vec![end(0)];
    stream.extend(write_heights(&mut wal, 1, 30));
    // The current height's records (no #ENDHEIGHT yet).
    for m in height_records(31) {
        wal.write(&m).unwrap();
        stream.push(m);
    }
    wal.flush_and_sync().unwrap();
    let max = wal.max_index().unwrap();
    assert!(max >= 3, "rotated {max} times");
    let mut want_names: Vec<String> = (0..max).map(|i| format!("wal.{i:03}")).collect();
    want_names.push("wal".into());
    want_names.sort();
    assert_eq!(names(&dir.path().join("cs.wal")), want_names);
    // Every rotated file ends at a record boundary and holds whole heights.
    for i in 0..max {
        let bz = fs::read(file_path_for_index(&path, i, max)).unwrap();
        let (msgs, n, err) = decode_all(&bz);
        assert!(err.is_none());
        assert_eq!(n, bz.len());
        assert!(matches!(msgs.last().unwrap().msg, WalMessage::EndHeight(_)));
    }
    for h in -1..=32 {
        assert_eq!(
            wal.search_for_end_height(h).unwrap(),
            after(&stream, h),
            "{h}"
        );
    }
    // Reopened (as after a restart), with Go's defaults.
    drop(wal);
    let mut wal = FileWal::open(&path, t(99)).unwrap();
    assert_eq!(wal.min_index().unwrap(), 0);
    assert_eq!(wal.max_index().unwrap(), max);
    for h in [0, 1, 17, 29, 30, 31] {
        assert_eq!(
            wal.search_for_end_height(h).unwrap(),
            after(&stream, h),
            "{h}"
        );
    }
}

#[test]
fn file_wal_prunes_oldest_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal");
    let mut wal = FileWal::open_with(&path, t(0), manual(1000, 4000)).unwrap();
    let mut stream = vec![end(0)];
    stream.extend(write_heights(&mut wal, 1, 40));
    let before = wal.group_info().unwrap();
    assert!(before.total_size >= 4000);
    for _ in 0..10 {
        wal.check_total_size_limit().unwrap();
    }
    let info = wal.group_info().unwrap();
    assert!(info.total_size < 4000, "{info:?}");
    assert!(info.min_index > 0);
    assert_eq!(info.max_index, before.max_index);
    // Pruned heights are gone; the recent ones replay.
    assert_eq!(wal.search_for_end_height(1).unwrap(), None);
    assert_eq!(
        wal.search_for_end_height(40).unwrap(),
        Some(vec![]),
        "the last marker survives"
    );
    assert_eq!(wal.search_for_end_height(39).unwrap(), after(&stream, 39));
}

/// A WAL written by builds before rotation (one head file, possibly
/// compacted so that it starts with a later `#ENDHEIGHT`, and maybe a
/// leftover `wal.tmp`) opens as a group of one file and keeps every byte.
#[test]
fn file_wal_opens_head_only_wal_from_older_builds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cs.wal").join("wal");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut old = Vec::new();
    let mut stream = vec![end(5)];
    for m in &stream {
        old.extend(encode(m).unwrap());
    }
    for m in height_records(6) {
        old.extend(encode(&m).unwrap());
        stream.push(m);
    }
    old.extend(encode(&end(6)).unwrap());
    stream.push(end(6));
    for m in height_records(7) {
        old.extend(encode(&m).unwrap());
        stream.push(m);
    }
    fs::write(&path, &old).unwrap();
    fs::write(path.with_extension("tmp"), b"stale").unwrap();

    let mut wal = FileWal::open_with(&path, t(0), manual(600, 0)).unwrap();
    // Not empty: no #ENDHEIGHT 0 is added.
    assert_eq!(fs::read(&path).unwrap(), old);
    assert_eq!(wal.min_index().unwrap(), 0);
    assert_eq!(wal.max_index().unwrap(), 0);
    // Replay at height 7 works as before.
    assert_eq!(wal.search_for_end_height(7).unwrap(), None);
    assert_eq!(wal.search_for_end_height(6).unwrap(), after(&stream, 6));
    // The node continues and the old head is rotated intact into wal.000.
    wal.write_sync(&end(7)).unwrap();
    stream.push(end(7));
    wal.check_head_size_limit().unwrap();
    stream.extend(write_heights(&mut wal, 8, 12));
    let rotated = fs::read(dir.path().join("cs.wal").join("wal.000")).unwrap();
    assert!(rotated.starts_with(&old));
    for h in 5..=12 {
        assert_eq!(
            wal.search_for_end_height(h).unwrap(),
            after(&stream, h),
            "{h}"
        );
    }
    assert!(dir.path().join("cs.wal").join("wal.tmp").exists());
}

/// Go's `OnStart` repair: the head is backed up to `wal.CORRUPTED` and
/// rewritten with its decodable prefix.
#[test]
fn file_wal_repair_like_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal");
    let mut wal = FileWal::open_with(&path, t(0), manual(800, 0)).unwrap();
    let mut stream = vec![end(0)];
    stream.extend(write_heights(&mut wal, 1, 10));
    let partial = height_records(11);
    for m in &partial {
        wal.write_sync(m).unwrap();
    }
    drop(wal);
    let original = fs::read(&path).unwrap();
    // Tear the last record.
    fs::write(&path, &original[..original.len() - 3]).unwrap();
    let mut wal = FileWal::open_with(&path, t(0), manual(800, 0)).unwrap();
    let Err(WalError::Corruption(e)) = wal.search_for_end_height(10) else {
        panic!("expected corruption");
    };
    assert!(e.starts_with("failed to read data: EOF"), "{e}");
    let found = wal.search(10).unwrap().unwrap();
    assert_eq!(found.msgs, partial[..partial.len() - 1].to_vec());
    wal.repair().unwrap();
    assert_eq!(
        fs::read(dir.path().join("wal.CORRUPTED")).unwrap(),
        original[..original.len() - 3].to_vec()
    );
    let enc_last = encode(partial.last().unwrap()).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        original[..original.len() - enc_last.len()].to_vec()
    );
    assert_eq!(
        wal.search_for_end_height(10).unwrap(),
        Some(partial[..partial.len() - 1].to_vec())
    );
    // The repaired WAL keeps working.
    wal.write_sync(&end(11)).unwrap();
    assert_eq!(wal.search_for_end_height(11).unwrap(), Some(vec![]));
}

/// A head emptied by repair gets `#ENDHEIGHT 0`, as Go's reloaded WAL does.
#[test]
fn file_wal_repair_of_garbage_head_restarts_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal");
    fs::write(&path, [0xffu8; 16]).unwrap();
    let mut wal = FileWal::open_with(&path, t(0), manual(0, 0)).unwrap();
    assert!(matches!(
        wal.search_for_end_height(0),
        Ok(None) // corruption before any marker is skipped
    ));
    wal.repair().unwrap();
    let (msgs, _, err) = decode_all(&fs::read(&path).unwrap());
    assert!(err.is_none());
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].msg, WalMessage::EndHeight(0));
    assert_eq!(wal.search_for_end_height(0).unwrap(), Some(vec![]));
}

/// A torn checksum (1-3 stray bytes) is the end of the stream for Go's
/// decoder, not corruption; 4-7 bytes cannot be read as a length.
#[test]
fn torn_tail_bytes() {
    let mut bz = Vec::new();
    for m in [end(0), end(1)] {
        bz.extend(encode(&m).unwrap());
    }
    for (extra, corrupt) in [(1, false), (3, false), (4, true), (7, true)] {
        let disk = Arc::new(Mutex::new(bz.clone()));
        disk.lock().unwrap().extend(vec![0xab; extra]);
        let mut wal = MemWal::open(disk, t(0)).unwrap();
        let r = wal.search_for_end_height(1);
        assert_eq!(r.is_err(), corrupt, "{extra}: {r:?}");
        assert_eq!(wal.search_for_end_height(0).is_err(), corrupt);
    }
}

/// `TestWALPeriodicSync`: buffered records reach the disk within the
/// flush interval.
#[test]
fn file_wal_periodic_flush() {
    let dir = tempfile::tempdir().unwrap();
    let opts = GroupOptions {
        flush_interval: Duration::from_millis(20),
        group_check_duration: Duration::ZERO,
        ..GroupOptions::default()
    };
    let mut wal = FileWal::open_with(dir.path().join("wal"), t(0), opts).unwrap();
    for m in height_records(1) {
        wal.write(&m).unwrap();
    }
    assert!(wal.buffered().unwrap() > 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while wal.buffered().unwrap() > 0 {
        assert!(Instant::now() < deadline, "not flushed");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The group ticker rotates the head and prunes on its own.
#[test]
fn file_wal_background_checks() {
    let dir = tempfile::tempdir().unwrap();
    let opts = GroupOptions {
        head_size_limit: 500,
        total_size_limit: 3000,
        group_check_duration: Duration::from_millis(5),
        flush_interval: Duration::from_millis(5),
    };
    let mut wal = FileWal::open_with(dir.path().join("wal"), t(0), opts).unwrap();
    for h in 1..=30 {
        for m in height_records(h) {
            wal.write(&m).unwrap();
        }
        wal.write_sync(&end(h)).unwrap();
        // Let the ticker rotate a full head before writing on (a slow
        // ticker would rotate one big file, which the 3000-byte total limit
        // then deletes: Go behaves the same).
        let deadline = Instant::now() + Duration::from_secs(10);
        while wal.group_info().unwrap().head_size >= 500 {
            assert!(Instant::now() < deadline, "no rotation");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let i = wal.group_info().unwrap();
        if i.max_index > 3 && i.min_index > 0 && i.total_size < 3000 {
            break;
        }
        assert!(Instant::now() < deadline, "{i:?} {:?}", names(dir.path()));
        std::thread::sleep(Duration::from_millis(5));
    }
    // Whatever the ticker did, the last height replays.
    assert_eq!(wal.search_for_end_height(30).unwrap(), Some(vec![]));
}

/// Dropping the WAL (`OnStop`) flushes what was buffered.
#[test]
fn file_wal_drop_flushes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal");
    let recs = height_records(1);
    {
        let mut wal = FileWal::open_with(&path, t(0), manual(0, 0)).unwrap();
        for m in &recs {
            wal.write(m).unwrap();
        }
    }
    let mut wal = FileWal::open_with(&path, t(0), manual(0, 0)).unwrap();
    assert_eq!(wal.search_for_end_height(0).unwrap(), Some(recs));
}

/// File modes as Go's `EnsureDir(dir, 0700)` and `AutoFile` (0600).
#[cfg(unix)]
#[test]
fn file_wal_modes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cs.wal").join("wal");
    let wal = FileWal::open_with(&path, t(0), manual(1, 0)).unwrap();
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&path), 0o600);
    wal.check_head_size_limit().unwrap();
    wal.check_head_size_limit().unwrap();
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&dir.path().join("cs.wal").join("wal.000")), 0o600);
}

// ---------------------------------------------------------------------------
// Go cross-compatibility vectors

#[derive(serde::Deserialize)]
struct Vectors {
    head_size_limit: u64,
    files: BTreeMap<String, String>,
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    files: Option<BTreeMap<String, String>>,
    #[serde(default)]
    mutations: Vec<Mutation>,
    info: VInfo,
    searches: Vec<VSearch>,
    #[serde(default)]
    total_size_limit: u64,
    #[serde(default)]
    remaining: Vec<String>,
    #[serde(default)]
    same_as_go: bool,
}

#[derive(serde::Deserialize)]
struct Mutation {
    file: String,
    kind: String,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    value: String,
    #[serde(default)]
    count: usize,
}

#[derive(serde::Deserialize)]
struct VInfo {
    min_index: usize,
    max_index: usize,
    total_size: u64,
    head_size: u64,
}

#[derive(serde::Deserialize, Debug, PartialEq, Eq)]
struct VSearch {
    height: i64,
    found: bool,
    count: usize,
    sha256: String,
    err: String,
}

fn vectors() -> Vectors {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/consensus/wal.json");
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}

fn write_files(dir: &Path, files: &BTreeMap<String, String>) {
    fs::create_dir_all(dir).unwrap();
    for (name, h) in files {
        fs::write(dir.join(name), hex::decode(h).unwrap()).unwrap();
    }
}

fn apply(dir: &Path, case: &Case) {
    if case.name == "fresh_head" {
        // The head was rotated away (as the vector generator does).
        let g = Group::open(dir.join("wal"), &manual(0, 0)).unwrap();
        let to = file_path_for_index(&dir.join("wal"), g.max_index, g.max_index + 1);
        drop(g);
        fs::rename(dir.join("wal"), to).unwrap();
        fs::write(dir.join("wal"), []).unwrap();
    }
    for m in &case.mutations {
        let p = dir.join(&m.file);
        let mut b = fs::read(&p).unwrap();
        match m.kind.as_str() {
            "xor" => b[m.offset] ^= hex::decode(&m.value).unwrap()[0],
            "truncate" => b.truncate(b.len() - m.count),
            "append" => b.extend(hex::decode(&m.value).unwrap()),
            k => panic!("mutation {k}"),
        }
        fs::write(&p, b).unwrap();
    }
}

/// What the Go vector program records for one search.
fn rust_search(wal: &mut FileWal, height: i64) -> VSearch {
    let found = wal.search(height).unwrap();
    let mut h = hash::Sha256Hasher::new();
    let (mut count, mut err) = (0, String::new());
    if let Some(f) = &found {
        for m in &f.msgs {
            h.update(&encode(m).unwrap());
        }
        count = f.msgs.len();
        if let Some(e) = &f.err {
            err = e.to_string();
        }
    }
    VSearch {
        height,
        found: found.is_some(),
        count,
        sha256: hex::encode(h.finalize()),
        err,
    }
}

fn check_case(dir: &Path, case: &Case) {
    let path = dir.join("wal");
    // The head is never empty in the vectors, so opening writes nothing.
    let before = names(dir);
    let mut wal = FileWal::open_with(&path, t(0), manual(0, 0)).unwrap();
    assert_eq!(names(dir), before, "{}", case.name);
    assert_eq!(
        wal.group_info().unwrap(),
        info(
            case.info.min_index,
            case.info.max_index,
            case.info.total_size,
            case.info.head_size
        ),
        "{}",
        case.name
    );
    for want in &case.searches {
        let got = rust_search(&mut wal, want.height);
        assert_eq!(&got, want, "case {}", case.name);
    }
}

/// Go → Rust: every group Go wrote (and corrupted variants) gives the
/// same search results, replayed records and errors in Rust; Go's
/// total-size pruning leaves the same files.
#[test]
fn wal_go_vectors() {
    let v = vectors();
    assert!(v.cases.len() >= 8);
    for case in &v.cases {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("cs.wal");
        write_files(&d, case.files.as_ref().unwrap_or(&v.files));
        apply(&d, case);
        if !case.remaining.is_empty() {
            let wal =
                FileWal::open_with(d.join("wal"), t(0), manual(0, case.total_size_limit)).unwrap();
            for _ in 0..20 {
                wal.check_total_size_limit().unwrap();
            }
            drop(wal);
            assert_eq!(names(&d), case.remaining, "{}", case.name);
        }
        check_case(&d, case);
    }
    assert!(v.cases.iter().any(|c| c.name == "rust_written"));
}

/// Rust writes the Go vector's record stream with the same rotation
/// points; the group directory is byte-identical to Go's.
fn write_rust_group(v: &Vectors, dir: &Path) {
    let mut stream = Vec::new();
    let mut idx: Vec<(usize, &String)> = v
        .files
        .iter()
        .map(|(n, h)| (indexed_file(n).unwrap_or(usize::MAX), h))
        .collect();
    idx.sort();
    for (_, h) in idx {
        stream.extend(hex::decode(h).unwrap());
    }
    let (msgs, n, err) = decode_all(&stream);
    assert!(err.is_none());
    assert_eq!(n, stream.len());
    assert_eq!(msgs[0].msg, WalMessage::EndHeight(0));
    let opts = manual(v.head_size_limit, 0);
    let mut wal = FileWal::open_with(dir.join("wal"), msgs[0].time, opts).unwrap();
    wal.check_head_size_limit().unwrap();
    for m in &msgs[1..] {
        wal.write(m).unwrap();
        if let WalMessage::EndHeight(_) = m.msg {
            wal.flush_and_sync().unwrap();
            wal.check_head_size_limit().unwrap();
        }
    }
    wal.flush_and_sync().unwrap();
}

fn read_files(dir: &Path) -> BTreeMap<String, String> {
    names(dir)
        .into_iter()
        .map(|n| {
            let h = hex::encode(fs::read(dir.join(&n)).unwrap());
            (n, h)
        })
        .collect()
}

/// Rust → Go: Rust's group is byte-identical to Go's, and Go's search
/// results on the Rust-written group (the `rust_written` case) match.
#[test]
fn wal_rust_group_matches_go() {
    let v = vectors();
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("cs.wal");
    write_rust_group(&v, &d);
    let rust = read_files(&d);
    assert_eq!(rust, v.files);
    let case = v.cases.iter().find(|c| c.name == "rust_written").unwrap();
    assert!(case.same_as_go, "Go saw a different Rust group");
    assert!(case.files.is_none());
    check_case(&d, case);
}

/// Writes the Rust group for the Go program's `-rust <dir>` mode.
#[test]
#[ignore = "vector generation: WAL_VECTORS_RUST_DIR=<dir>"]
fn wal_vectors_write_rust_group() {
    let dir = std::env::var("WAL_VECTORS_RUST_DIR").expect("WAL_VECTORS_RUST_DIR");
    write_rust_group(&vectors(), Path::new(&dir));
}
