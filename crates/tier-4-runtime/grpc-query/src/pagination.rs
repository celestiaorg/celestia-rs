//! `types/query` pagination: `Paginate`, `FilteredPaginate`,
//! `GenericFilteredPaginate` (prefix-store based) and
//! `CollectionFilteredPaginate` (collections based). Each module uses one
//! of them per endpoint and their edge cases differ (next key, totals,
//! offsets past the end), so they are ported separately.

use celestia_proto::cosmos::base::query::v1beta1::{PageRequest, PageResponse};
use sdk::Context;

use crate::error::{QResult, QueryError};

/// `query.DefaultLimit`.
pub const DEFAULT_LIMIT: u64 = 100;

/// Error of a request with both offset and key.
pub const BOTH_OFFSET_AND_KEY: &str = "invalid request, either offset or key is expected, got both";

/// A raw entry: key relative to the iterated prefix, value.
pub type Entry = (Vec<u8>, Vec<u8>);

/// `storetypes.PrefixEndBytes` (`None` for "no upper bound").
#[must_use]
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    if prefix.is_empty() {
        return None;
    }
    sdk::context::prefix_end(prefix)
}

/// Entries of `store` under `prefix` in `[start, end)` (both relative to
/// `prefix`; `end == None` means up to the end of the prefix), keys
/// returned relative to `prefix`, without gas.
fn range_raw(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    start: Option<&[u8]>,
    end: Option<&[u8]>,
    reverse: bool,
) -> QResult<Option<Vec<Entry>>> {
    let abs_start = [prefix, start.unwrap_or_default()].concat();
    let abs_end = match end {
        Some(e) => Some([prefix, e].concat()),
        None => prefix_end(prefix),
    };
    if let Some(e) = &abs_end
        && abs_start.as_slice() > e.as_slice()
    {
        return Ok(None);
    }
    let items = ctx
        .multistore()
        .range(
            store,
            (!abs_start.is_empty()).then_some(abs_start.as_slice()),
            abs_end.as_deref(),
            reverse,
        )
        .map_err(|e| QueryError::Plain(e.to_string()))?;
    Ok(Some(
        items
            .into_iter()
            .map(|(k, v)| (k[prefix.len()..].to_vec(), v))
            .collect(),
    ))
}

/// Entries of `store` under `prefix` in `[start, end)` (both relative to
/// `prefix`; `end == None` means up to the end of the prefix), keys
/// returned relative to `prefix`. Gas is charged as a `gaskv` iterator
/// walked to its end (`Walk`, `GetAll`, `IterateX`).
pub fn range(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    start: Option<&[u8]>,
    end: Option<&[u8]>,
    reverse: bool,
) -> QResult<Vec<Entry>> {
    let Some(items) = range_raw(ctx, store, prefix, start, end, reverse)? else {
        return Ok(Vec::new());
    };
    let mut c = Cursor::open(ctx, store, prefix.len(), items)?;
    while c.valid() {
        c.next(ctx)?;
    }
    Ok(c.items)
}

/// A `gaskv` iterator over a snapshot: creation and every `Next` charge
/// the seek gas of the entry the iterator is positioned at.
struct Cursor<'s> {
    store: &'s str,
    /// Length of the store prefix the entry keys are relative to.
    plen: usize,
    items: Vec<Entry>,
    pos: usize,
}

impl<'s> Cursor<'s> {
    fn open(
        ctx: &mut Context<'_>,
        store: &'s str,
        plen: usize,
        items: Vec<Entry>,
    ) -> QResult<Self> {
        let c = Self {
            store,
            plen,
            items,
            pos: 0,
        };
        c.seek(ctx)?;
        Ok(c)
    }

    fn seek(&self, ctx: &mut Context<'_>) -> QResult<()> {
        let entry = self
            .items
            .get(self.pos)
            .map(|(k, v)| (self.plen + k.len(), v.len()));
        ctx.charge_iter_seek(self.store, entry)
            .map_err(QueryError::Sdk)
    }

    fn valid(&self) -> bool {
        self.pos < self.items.len()
    }

    /// The current entry (cloned: callbacks need the context).
    fn cur(&self) -> Entry {
        self.items.get(self.pos).cloned().unwrap_or_default()
    }

    fn next(&mut self, ctx: &mut Context<'_>) -> QResult<()> {
        self.seek(ctx)?;
        if self.valid() {
            self.pos += 1;
        }
        Ok(())
    }
}

/// `initPageRequestDefaults`.
#[must_use]
pub fn defaults(page: Option<&PageRequest>) -> PageRequest {
    let mut p = page.cloned().unwrap_or_default();
    if p.limit == 0 {
        p.limit = DEFAULT_LIMIT;
        p.count_total = true;
    }
    p
}

/// `getIterator` over a prefix store.
fn prefix_iter<'s>(
    ctx: &mut Context<'_>,
    store: &'s str,
    prefix: &[u8],
    start: Option<&[u8]>,
    reverse: bool,
) -> QResult<Cursor<'s>> {
    let plen = prefix.len();
    if reverse {
        let mut end = None;
        if let Some(start) = start {
            let fwd = range_raw(ctx, store, prefix, Some(start), None, false)?.unwrap_or_default();
            let mut probe = Cursor::open(ctx, store, plen, fwd)?;
            if probe.valid() {
                probe.next(ctx)?;
                if probe.valid() {
                    end = Some(probe.cur().0);
                }
            }
        }
        let items = range_raw(ctx, store, prefix, None, end.as_deref(), true)?.unwrap_or_default();
        return Cursor::open(ctx, store, plen, items);
    }
    let items = range_raw(ctx, store, prefix, start, None, false)?.unwrap_or_default();
    Cursor::open(ctx, store, plen, items)
}

/// `query.Paginate` over `store` restricted to `prefix`; `on_result`
/// receives keys relative to `prefix`.
pub fn paginate(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    page: Option<&PageRequest>,
    mut on_result: impl FnMut(&mut Context<'_>, &[u8], &[u8]) -> QResult<()>,
) -> QResult<PageResponse> {
    let p = defaults(page);
    let key = (!p.key.is_empty()).then_some(p.key.as_slice());
    if p.offset > 0 && key.is_some() {
        return Err(QueryError::Plain(BOTH_OFFSET_AND_KEY.into()));
    }
    let mut it = prefix_iter(ctx, store, prefix, key, p.reverse)?;
    let mut count = 0u64;
    let mut next_key = Vec::new();
    if key.is_some() {
        while it.valid() {
            let (k, v) = it.cur();
            if count == p.limit {
                next_key = k;
                break;
            }
            on_result(ctx, &k, &v)?;
            count += 1;
            it.next(ctx)?;
        }
        return Ok(PageResponse { next_key, total: 0 });
    }
    let end = p.offset.saturating_add(p.limit);
    while it.valid() {
        let (k, v) = it.cur();
        count += 1;
        if count > p.offset {
            if count <= end {
                on_result(ctx, &k, &v)?;
            } else if count == end + 1 {
                next_key = k;
                if !p.count_total {
                    break;
                }
            }
        }
        it.next(ctx)?;
    }
    Ok(PageResponse {
        next_key,
        total: if p.count_total { count } else { 0 },
    })
}

/// `query.FilteredPaginate`: `on_result(key, value, accumulate)` returns
/// whether the entry is a hit.
pub fn filtered_paginate(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    page: Option<&PageRequest>,
    mut on_result: impl FnMut(&mut Context<'_>, &[u8], &[u8], bool) -> QResult<bool>,
) -> QResult<PageResponse> {
    let p = defaults(page);
    let key = (!p.key.is_empty()).then_some(p.key.as_slice());
    if p.offset > 0 && key.is_some() {
        return Err(QueryError::Plain(BOTH_OFFSET_AND_KEY.into()));
    }
    let mut it = prefix_iter(ctx, store, prefix, key, p.reverse)?;
    let mut hits = 0u64;
    let mut next_key: Option<Vec<u8>> = None;
    if key.is_some() {
        while it.valid() {
            let (k, v) = it.cur();
            if hits == p.limit {
                next_key = Some(k);
                break;
            }
            if on_result(ctx, &k, &v, true)? {
                hits += 1;
            }
            it.next(ctx)?;
        }
        return Ok(PageResponse {
            next_key: next_key.unwrap_or_default(),
            total: 0,
        });
    }
    let end = p.offset.saturating_add(p.limit);
    while it.valid() {
        let (k, v) = it.cur();
        let accumulate = hits >= p.offset && hits < end;
        if on_result(ctx, &k, &v, accumulate)? {
            hits += 1;
        }
        if hits == end + 1 {
            if next_key.is_none() {
                next_key = Some(k);
            }
            if !p.count_total {
                break;
            }
        }
        it.next(ctx)?;
    }
    Ok(PageResponse {
        next_key: next_key.unwrap_or_default(),
        total: if p.count_total { hits } else { 0 },
    })
}

/// `query.GenericFilteredPaginate`: `on_result` returns `Some(item)` for a
/// hit (Go: a non-empty message) and `None` to skip.
pub fn generic_filtered_paginate<T>(
    ctx: &mut Context<'_>,
    store: &str,
    prefix: &[u8],
    page: Option<&PageRequest>,
    mut on_result: impl FnMut(&mut Context<'_>, &[u8], &[u8]) -> QResult<Option<T>>,
) -> QResult<(Vec<T>, PageResponse)> {
    let p = defaults(page);
    let key = (!p.key.is_empty()).then_some(p.key.as_slice());
    if p.offset > 0 && key.is_some() {
        return Err(QueryError::Plain(BOTH_OFFSET_AND_KEY.into()));
    }
    let mut it = prefix_iter(ctx, store, prefix, key, p.reverse)?;
    let mut results = Vec::new();
    let mut hits = 0u64;
    let mut next_key: Option<Vec<u8>> = None;
    if key.is_some() {
        while it.valid() {
            let (k, v) = it.cur();
            if hits == p.limit {
                next_key = Some(k);
                break;
            }
            if let Some(t) = on_result(ctx, &k, &v)? {
                results.push(t);
                hits += 1;
            }
            it.next(ctx)?;
        }
        return Ok((
            results,
            PageResponse {
                next_key: next_key.unwrap_or_default(),
                total: 0,
            },
        ));
    }
    let end = p.offset.saturating_add(p.limit);
    while it.valid() {
        let (k, v) = it.cur();
        if let Some(t) = on_result(ctx, &k, &v)? {
            if hits >= p.offset && hits < end {
                results.push(t);
            }
            hits += 1;
        }
        if hits == end + 1 {
            if next_key.is_none() {
                next_key = Some(k);
            }
            if !p.count_total {
                break;
            }
        }
        it.next(ctx)?;
    }
    Ok((
        results,
        PageResponse {
            next_key: next_key.unwrap_or_default(),
            total: if p.count_total { hits } else { 0 },
        },
    ))
}

/// A collection (`collections.Map`) in `store` under `prefix`.
#[derive(Debug, Clone, Copy)]
pub struct Collection<'a> {
    /// Store name.
    pub store: &'a str,
    /// Collection prefix.
    pub prefix: &'a [u8],
}

/// Predicate of [`collection_filtered_paginate`].
pub type Predicate<'a> = &'a mut dyn FnMut(&mut Context<'_>, &[u8], &[u8]) -> QResult<bool>;

/// `query.CollectionPaginate`.
pub fn collection_paginate<T>(
    ctx: &mut Context<'_>,
    coll: Collection<'_>,
    page: Option<&PageRequest>,
    key_prefix: Option<&[u8]>,
    transform: impl FnMut(&mut Context<'_>, &[u8], &[u8]) -> QResult<T>,
) -> QResult<(Vec<T>, PageResponse)> {
    collection_filtered_paginate(ctx, coll, page, key_prefix, None, transform)
}

/// `query.CollectionFilteredPaginate`. Keys passed to the callbacks are the
/// encoded collection keys (relative to the collection prefix);
/// `key_prefix` is `WithCollectionPaginationPairPrefix`'s encoded prefix.
pub fn collection_filtered_paginate<T>(
    ctx: &mut Context<'_>,
    coll: Collection<'_>,
    page: Option<&PageRequest>,
    key_prefix: Option<&[u8]>,
    mut predicate: Option<Predicate<'_>>,
    mut transform: impl FnMut(&mut Context<'_>, &[u8], &[u8]) -> QResult<T>,
) -> QResult<(Vec<T>, PageResponse)> {
    let p = defaults(page);
    let key = (!p.key.is_empty()).then_some(p.key.as_slice());
    if p.offset > 0 && key.is_some() {
        return Err(QueryError::Plain(BOTH_OFFSET_AND_KEY.into()));
    }
    let prefix = key_prefix.unwrap_or_default();
    // IterateRaw with start > end: ErrInvalidIterator → empty page response.
    let Some(mut it) = coll_iter(ctx, coll, prefix, key.unwrap_or_default(), p.reverse)? else {
        return Ok((Vec::new(), PageResponse::default()));
    };
    let mut results = Vec::new();
    let mut next_key = Vec::new();
    let mut include = |ctx: &mut Context<'_>, k: &[u8], v: &[u8]| -> QResult<bool> {
        match predicate.as_mut() {
            None => Ok(true),
            Some(f) => f(ctx, k, v),
        }
    };
    if key.is_some() {
        let mut count = 0u64;
        while it.valid() {
            let (k, v) = it.cur();
            if count == p.limit {
                next_key = k;
                break;
            }
            if include(ctx, &k, &v)? {
                results.push(transform(ctx, &k, &v)?);
            }
            count += 1;
            it.next(ctx)?;
        }
        return Ok((results, strip(next_key, prefix, 0)));
    }
    // advanceIter.
    for _ in 0..p.offset {
        if !it.valid() {
            return Ok((Vec::new(), PageResponse::default()));
        }
        it.next(ctx)?;
    }
    let mut count = 0u64;
    while it.valid() {
        let (k, v) = it.cur();
        if count < p.limit {
            if include(ctx, &k, &v)? {
                results.push(transform(ctx, &k, &v)?);
                count += 1;
            }
        } else {
            if count == p.limit {
                next_key.clone_from(&k);
                if !p.count_total {
                    return Ok((results, strip(next_key, prefix, 0)));
                }
            }
            if include(ctx, &k, &v)? {
                count += 1;
            }
        }
        it.next(ctx)?;
    }
    let total = if p.count_total { count + p.offset } else { 0 };
    Ok((results, strip(next_key, prefix, total)))
}

fn strip(next_key: Vec<u8>, prefix: &[u8], total: u64) -> PageResponse {
    let next_key = if !next_key.is_empty() && !prefix.is_empty() {
        next_key[prefix.len().min(next_key.len())..].to_vec()
    } else {
        next_key
    };
    PageResponse { next_key, total }
}

/// `getCollIter`: entries keyed relative to the collection prefix (`None`:
/// `ErrInvalidIterator`).
fn coll_iter<'s>(
    ctx: &mut Context<'_>,
    coll: Collection<'s>,
    prefix: &[u8],
    start: &[u8],
    reverse: bool,
) -> QResult<Option<Cursor<'s>>> {
    let items = if reverse {
        let end = prefix_end(&[prefix, start].concat());
        range_raw(
            ctx,
            coll.store,
            coll.prefix,
            (!prefix.is_empty()).then_some(prefix),
            end.as_deref(),
            true,
        )?
    } else {
        let (start, end) = if prefix.is_empty() {
            (start.to_vec(), None)
        } else {
            ([prefix, start].concat(), prefix_end(prefix))
        };
        range_raw(
            ctx,
            coll.store,
            coll.prefix,
            (!start.is_empty()).then_some(start.as_slice()),
            end.as_deref(),
            false,
        )?
    };
    items
        .map(|items| Cursor::open(ctx, coll.store, coll.prefix.len(), items))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn setup(n: u8) -> MultiStore {
        let mut ms = MultiStore::new();
        ms.mount("s", Kind::Iavl).unwrap();
        {
            let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
            for i in 0..n {
                ctx.set("s", vec![1, i], vec![i]).unwrap();
            }
            ctx.set("s", vec![2, 0], vec![0]).unwrap();
        }
        ms
    }

    #[test]
    fn paginate_offset_limit_total() {
        let mut ms = setup(5);
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let mut got = Vec::new();
        let page = PageRequest {
            offset: 1,
            limit: 2,
            count_total: true,
            ..Default::default()
        };
        let r = paginate(&mut ctx, "s", &[1], Some(&page), |_, k, _| {
            got.push(k.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(got, vec![vec![1], vec![2]]);
        assert_eq!(r.next_key, vec![3]);
        assert_eq!(r.total, 5);
    }

    #[test]
    fn paginate_reverse_key() {
        let mut ms = setup(5);
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let mut got = Vec::new();
        let page = PageRequest {
            key: vec![2],
            limit: 2,
            reverse: true,
            ..Default::default()
        };
        let r = paginate(&mut ctx, "s", &[1], Some(&page), |_, k, _| {
            got.push(k[0]);
            Ok(())
        })
        .unwrap();
        assert_eq!(got, vec![2, 1]);
        assert_eq!(r.next_key, vec![0]);
    }

    #[test]
    fn collection_offset_past_end() {
        let mut ms = setup(3);
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let coll = Collection {
            store: "s",
            prefix: &[1],
        };
        let page = PageRequest {
            offset: 4,
            limit: 1,
            ..Default::default()
        };
        let (r, p) =
            collection_paginate(&mut ctx, coll, Some(&page), None, |_, k, _| Ok(k[0])).unwrap();
        assert!(r.is_empty());
        assert_eq!(p, PageResponse::default());
        let (r, p) = collection_paginate(&mut ctx, coll, None, None, |_, k, _| Ok(k[0])).unwrap();
        assert_eq!(r, vec![0, 1, 2]);
        assert_eq!(p.total, 3);
    }
}
