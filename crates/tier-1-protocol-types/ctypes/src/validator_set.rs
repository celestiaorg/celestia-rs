//! `ValidatorSet` (`types/validator_set.go`): proposer-priority rotation,
//! change sets, hashing.
//!
//! Go keeps `Proposer` as a pointer. After `IncrementProposerPriority` (or
//! `GetProposer` / `ValidatorSetFromExistingValidators`) it aliases a member
//! of `Validators` and follows that member's later priority changes; once
//! the member is replaced (`UpdateWithChangeSet` of its address) or removed,
//! the pointer keeps the old object. This port models exactly that: the
//! proposer is either a live reference to a member (by address) or a
//! detached snapshot. `Copy` shares the pointer in Go; here the copy holds a
//! snapshot taken at copy time (identical unless the source set is mutated
//! afterwards).

use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::merkle_proof::hash_from_byte_slices;
use crate::proto::types as pb;
use crate::validator::{Validator, validator_list_go};

/// `MaxTotalVotingPower` = `MaxInt64 / 8`.
pub const MAX_TOTAL_VOTING_POWER: i64 = i64::MAX / 8;
/// `PriorityWindowSizeFactor`.
pub const PRIORITY_WINDOW_SIZE_FACTOR: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProposerRef {
    /// Aliases the member with this address.
    Member(Vec<u8>),
    /// A validator object no longer (or never) in the set.
    Detached(Validator),
}

/// A validator set for one height, sorted by voting power (desc) then
/// address (asc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    /// Validators. Mutating this directly bypasses the cached total power
    /// (as in Go).
    pub validators: Vec<Validator>,
    proposer: Option<ProposerRef>,
    total_voting_power: i64,
    all_keys_have_same_type: bool,
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self {
            validators: Vec::new(),
            proposer: None,
            total_voting_power: 0,
            all_keys_have_same_type: true,
        }
    }
}

fn safe_add(a: i64, b: i64) -> (i64, bool) {
    match a.checked_add(b) {
        Some(c) => (c, false),
        None => (-1, true),
    }
}

fn safe_sub(a: i64, b: i64) -> (i64, bool) {
    match a.checked_sub(b) {
        Some(c) => (c, false),
        None => (-1, true),
    }
}

/// `safeAddClip`.
#[must_use]
pub fn safe_add_clip(a: i64, b: i64) -> i64 {
    let (c, overflow) = safe_add(a, b);
    if overflow {
        return if b < 0 { i64::MIN } else { i64::MAX };
    }
    c
}

/// `safeSubClip`.
#[must_use]
pub fn safe_sub_clip(a: i64, b: i64) -> i64 {
    let (c, overflow) = safe_sub(a, b);
    if overflow {
        return if b > 0 { i64::MIN } else { i64::MAX };
    }
    c
}

/// `safeMul`: `(product, overflow)`.
#[must_use]
pub fn safe_mul(a: i64, b: i64) -> (i64, bool) {
    if a == 0 || b == 0 {
        return (0, false);
    }
    let abs_b = if b < 0 { b.wrapping_neg() } else { b };
    let abs_a = if a < 0 { a.wrapping_neg() } else { a };
    if abs_a > i64::MAX / abs_b {
        return (0, true);
    }
    (a.wrapping_mul(b), false)
}

/// Sort by voting power (desc), then address (asc): `ValidatorsByVotingPower`.
pub fn sort_by_voting_power(vals: &mut [Validator]) {
    vals.sort_by(|a, b| {
        b.voting_power
            .cmp(&a.voting_power)
            .then_with(|| a.address.cmp(&b.address))
    });
}

/// Sort by address: `ValidatorsByAddress`.
pub fn sort_by_address(vals: &mut [Validator]) {
    vals.sort_by(|a, b| a.address.cmp(&b.address));
}

impl ValidatorSet {
    /// `NewValidatorSet`: builds the set through `updateWithChangeSet` and
    /// increments priorities once (Go panics where this returns an error).
    pub fn new(list: &[Validator]) -> Result<Self> {
        let mut vals = Self::default();
        vals.update_with_change_set_inner(list, false)
            .map_err(|e| e.wrap("Cannot create validator set: "))?;
        if !list.is_empty() {
            vals.increment_proposer_priority(1)?;
        }
        Ok(vals)
    }

    /// `ValidatorSet.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.is_nil_or_empty() {
            return Err(Error::msg("validator set is nil or empty"));
        }
        for (idx, val) in self.validators.iter().enumerate() {
            val.validate_basic()
                .map_err(|e| e.wrap(&format!("invalid validator #{idx}: ")))?;
        }
        let Some(proposer) = self.proposer_ref() else {
            return Err(Error::msg(
                "proposer failed validate basic, error: nil validator",
            ));
        };
        proposer
            .validate_basic()
            .map_err(|e| e.wrap("proposer failed validate basic, error: "))?;
        if self
            .validators
            .iter()
            .any(|v| v.address == proposer.address)
        {
            return Ok(());
        }
        Err(Error::ProposerNotInVals)
    }

    /// `IsNilOrEmpty`.
    #[must_use]
    pub fn is_nil_or_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// `CopyIncrementProposerPriority`.
    pub fn copy_increment_proposer_priority(&self, times: i32) -> Result<Self> {
        let mut cp = self.copy();
        cp.increment_proposer_priority(times)?;
        Ok(cp)
    }

    /// `IncrementProposerPriority` (errors where Go panics).
    pub fn increment_proposer_priority(&mut self, times: i32) -> Result<()> {
        if self.is_nil_or_empty() {
            return Err(Error::msg("empty validator set"));
        }
        if times <= 0 {
            return Err(Error::msg(
                "Cannot call IncrementProposerPriority with non-positive times",
            ));
        }
        let diff_max = PRIORITY_WINDOW_SIZE_FACTOR.wrapping_mul(self.total_voting_power());
        self.rescale_priorities(diff_max)?;
        self.shift_by_avg_proposer_priority();
        let mut proposer = 0;
        for _ in 0..times {
            proposer = self.increment_proposer_priority_once();
        }
        self.proposer = Some(ProposerRef::Member(
            self.validators[proposer].address.clone(),
        ));
        Ok(())
    }

    /// `RescalePriorities` (errors where Go panics).
    pub fn rescale_priorities(&mut self, diff_max: i64) -> Result<()> {
        if self.is_nil_or_empty() {
            return Err(Error::msg("empty validator set"));
        }
        if diff_max <= 0 {
            return Ok(());
        }
        let diff = self.max_min_priority_diff();
        let ratio = diff.wrapping_add(diff_max).wrapping_sub(1) / diff_max;
        if diff > diff_max {
            if ratio == 0 {
                return Err(Error::msg("runtime error: integer divide by zero"));
            }
            for val in &mut self.validators {
                val.proposer_priority = val.proposer_priority.wrapping_div(ratio);
            }
        }
        Ok(())
    }

    fn increment_proposer_priority_once(&mut self) -> usize {
        for val in &mut self.validators {
            val.proposer_priority = safe_add_clip(val.proposer_priority, val.voting_power);
        }
        let mostest = self.val_with_most_priority();
        let tvp = self.total_voting_power();
        let v = &mut self.validators[mostest];
        v.proposer_priority = safe_sub_clip(v.proposer_priority, tvp);
        mostest
    }

    fn compute_avg_proposer_priority(&self) -> i64 {
        if self.validators.is_empty() {
            return 0;
        }
        let n = i128::try_from(self.validators.len()).unwrap_or(1);
        let sum: i128 = self
            .validators
            .iter()
            .map(|v| i128::from(v.proposer_priority))
            .sum();
        // big.Int.Div is Euclidean division.
        i64::try_from(sum.div_euclid(n)).unwrap_or(0)
    }

    fn max_min_priority_diff(&self) -> i64 {
        let mut max = i64::MIN;
        let mut min = i64::MAX;
        for v in &self.validators {
            min = min.min(v.proposer_priority);
            max = max.max(v.proposer_priority);
        }
        let diff = max.wrapping_sub(min);
        if diff < 0 { diff.wrapping_neg() } else { diff }
    }

    fn val_with_most_priority(&self) -> usize {
        let mut res = 0;
        for i in 1..self.validators.len() {
            if !self.validators[res].wins_over(&self.validators[i]) {
                res = i;
            }
        }
        res
    }

    fn shift_by_avg_proposer_priority(&mut self) {
        let avg = self.compute_avg_proposer_priority();
        for v in &mut self.validators {
            v.proposer_priority = safe_sub_clip(v.proposer_priority, avg);
        }
    }

    /// `ValidatorSet.Copy` (see the module docs for the proposer).
    #[must_use]
    pub fn copy(&self) -> Self {
        Self {
            validators: self.validators.clone(),
            proposer: self.proposer_ref().cloned().map(ProposerRef::Detached),
            total_voting_power: self.total_voting_power,
            all_keys_have_same_type: self.all_keys_have_same_type,
        }
    }

    /// `HasAddress`.
    #[must_use]
    pub fn has_address(&self, address: &[u8]) -> bool {
        self.validators.iter().any(|v| v.address == address)
    }

    /// `GetByAddress`: index and a copy, or `None` (Go's -1, nil).
    #[must_use]
    pub fn get_by_address(&self, address: &[u8]) -> Option<(i32, Validator)> {
        self.validators
            .iter()
            .position(|v| v.address == address)
            .map(|i| {
                (
                    i32::try_from(i).unwrap_or(i32::MAX),
                    self.validators[i].clone(),
                )
            })
    }

    /// `GetByIndex`.
    #[must_use]
    pub fn get_by_index(&self, index: i32) -> Option<(Vec<u8>, Validator)> {
        let v = self.validators.get(usize::try_from(index).ok()?)?;
        Some((v.address.clone(), v.clone()))
    }

    /// `Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.validators.len()
    }

    fn compute_total_voting_power(&self) -> Result<i64> {
        let mut sum = 0i64;
        for v in &self.validators {
            sum = safe_add_clip(sum, v.voting_power);
            if sum > MAX_TOTAL_VOTING_POWER {
                return Err(Error::msg(format!(
                    "total voting power {sum} exceeds maximum {MAX_TOTAL_VOTING_POWER}"
                )));
            }
        }
        Ok(sum)
    }

    fn update_total_voting_power(&mut self) -> Result<()> {
        self.total_voting_power = self.compute_total_voting_power()?;
        Ok(())
    }

    /// `TotalVotingPowerSafe`.
    pub fn total_voting_power_safe(&mut self) -> Result<i64> {
        if self.total_voting_power == 0 {
            self.update_total_voting_power()?;
        }
        Ok(self.total_voting_power)
    }

    /// `TotalVotingPower` (cached; recomputed while the cache is 0). Go
    /// panics on overflow; this then returns the clipped sum.
    #[must_use]
    pub fn total_voting_power(&self) -> i64 {
        if self.total_voting_power != 0 {
            return self.total_voting_power;
        }
        self.compute_total_voting_power().unwrap_or_else(|_| {
            self.validators
                .iter()
                .fold(0, |s, v| safe_add_clip(s, v.voting_power))
        })
    }

    fn proposer_ref(&self) -> Option<&Validator> {
        match self.proposer.as_ref()? {
            ProposerRef::Member(addr) => self.validators.iter().find(|v| &v.address == addr),
            ProposerRef::Detached(v) => Some(v),
        }
    }

    /// The stored proposer without computing one (Go's `vals.Proposer`).
    #[must_use]
    pub fn proposer(&self) -> Option<&Validator> {
        self.proposer_ref()
    }

    /// `GetProposer`: computes and stores the proposer when unset.
    pub fn get_proposer(&mut self) -> Option<Validator> {
        if self.validators.is_empty() {
            return None;
        }
        if self.proposer.is_none() {
            let idx = self.find_proposer();
            self.proposer = Some(ProposerRef::Member(self.validators[idx].address.clone()));
        }
        self.proposer_ref().cloned()
    }

    /// `GetProposer` without storing the result.
    #[must_use]
    pub fn peek_proposer(&self) -> Option<Validator> {
        if self.validators.is_empty() {
            return None;
        }
        if self.proposer.is_none() {
            return Some(self.validators[self.find_proposer()].clone());
        }
        self.proposer_ref().cloned()
    }

    fn find_proposer(&self) -> usize {
        let mut res = 0;
        for i in 1..self.validators.len() {
            if self.validators[i].address != self.validators[res].address
                && !self.validators[res].wins_over(&self.validators[i])
            {
                res = i;
            }
        }
        res
    }

    /// `ValidatorSet.Hash`: Merkle root of `Validator.Bytes`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        let bzs: Vec<Vec<u8>> = self.validators.iter().map(Validator::bytes).collect();
        hash_from_byte_slices(&bzs)
    }

    /// `ProposerPriorityHash` (`None` for an empty set).
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn proposer_priority_hash(&self) -> Option<Vec<u8>> {
        if self.validators.is_empty() {
            return None;
        }
        let mut buf = Vec::new();
        for v in &self.validators {
            let mut ux = (v.proposer_priority as u64) << 1;
            if v.proposer_priority < 0 {
                ux = !ux;
            }
            while ux >= 0x80 {
                buf.push((ux as u8) | 0x80);
                ux >>= 7;
            }
            buf.push(ux as u8);
        }
        Some(hash::sha256(&buf).to_vec())
    }

    /// `UpdateWithChangeSet`: applies updates (power > 0) and removals
    /// (power 0) atomically; on error the set is unchanged.
    pub fn update_with_change_set(&mut self, changes: &[Validator]) -> Result<()> {
        self.update_with_change_set_inner(changes, true)
    }

    fn update_with_change_set_inner(
        &mut self,
        changes: &[Validator],
        allow_deletes: bool,
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let (mut updates, deletes) = process_changes(changes)?;
        if !allow_deletes && !deletes.is_empty() {
            return Err(Error::msg(format!(
                "cannot process validators with voting power 0: {}",
                validator_list_go(&deletes)
            )));
        }
        if num_new_validators(&updates, self) == 0 && self.validators.len() == deletes.len() {
            return Err(Error::msg(
                "applying the validator changes would result in empty set",
            ));
        }
        let removed_voting_power = verify_removals(&deletes, self)?;
        let tvp_after_updates_before_removals =
            verify_updates(&updates, self, removed_voting_power)?;
        compute_new_priorities(&mut updates, self, tvp_after_updates_before_removals);
        self.apply_updates(updates);
        self.apply_removals(&deletes);
        self.check_all_keys_have_same_type();
        self.update_total_voting_power()?;
        self.rescale_priorities(PRIORITY_WINDOW_SIZE_FACTOR * self.total_voting_power())?;
        self.shift_by_avg_proposer_priority();
        sort_by_voting_power(&mut self.validators);
        Ok(())
    }

    fn detach_proposer_if(&mut self, addr: &[u8]) {
        if let Some(ProposerRef::Member(a)) = &self.proposer
            && a == addr
            && let Some(v) = self.validators.iter().find(|v| v.address == addr)
        {
            self.proposer = Some(ProposerRef::Detached(v.clone()));
        }
    }

    fn apply_updates(&mut self, updates: Vec<Validator>) {
        for u in &updates {
            self.detach_proposer_if(&u.address);
        }
        let mut existing = core::mem::take(&mut self.validators);
        sort_by_address(&mut existing);
        let mut merged = Vec::with_capacity(existing.len() + updates.len());
        let mut e = existing.into_iter().peekable();
        let mut u = updates.into_iter().peekable();
        while let (Some(ex), Some(up)) = (e.peek(), u.peek()) {
            if ex.address < up.address {
                merged.extend(e.next());
            } else {
                if ex.address == up.address {
                    e.next();
                }
                merged.extend(u.next());
            }
        }
        merged.extend(e);
        merged.extend(u);
        self.validators = merged;
    }

    fn apply_removals(&mut self, deletes: &[Validator]) {
        for d in deletes {
            self.detach_proposer_if(&d.address);
        }
        self.validators
            .retain(|v| !deletes.iter().any(|d| d.address == v.address));
    }

    fn check_all_keys_have_same_type(&mut self) {
        self.all_keys_have_same_type = match self.validators.first() {
            None => true,
            Some(first) => {
                let t = first.pub_key.key_type();
                self.validators.iter().all(|v| v.pub_key.key_type() == t)
            }
        };
    }

    /// `AllKeysHaveSameType`.
    #[must_use]
    pub fn all_keys_have_same_type(&self) -> bool {
        self.all_keys_have_same_type
    }

    /// `ValidatorSet.ToProto`.
    pub fn to_proto(&self) -> Result<pb::ValidatorSet> {
        if self.is_nil_or_empty() {
            return Ok(pb::ValidatorSet::default());
        }
        let proposer = self
            .proposer_ref()
            .ok_or_else(|| Error::msg("toProto: validatorSet proposer error: nil validator"))?;
        Ok(pb::ValidatorSet {
            validators: self.validators.iter().map(Validator::to_proto).collect(),
            proposer: Some(proposer.to_proto()),
            total_voting_power: 0,
        })
    }

    /// `ValidatorSetFromProto` (validates).
    pub fn from_proto(vp: &pb::ValidatorSet) -> Result<Self> {
        let validators = vp
            .validators
            .iter()
            .map(Validator::from_proto)
            .collect::<Result<Vec<_>>>()?;
        let mut vals = Self {
            validators,
            ..Self::default()
        };
        vals.check_all_keys_have_same_type();
        let proposer = match &vp.proposer {
            None => {
                return Err(Error::msg(
                    "fromproto: validatorSet proposer error: nil validator",
                ));
            }
            Some(p) => Validator::from_proto(p)
                .map_err(|e| e.wrap("fromproto: validatorSet proposer error: "))?,
        };
        vals.proposer = Some(ProposerRef::Detached(proposer));
        vals.total_voting_power_safe()?;
        vals.validate_basic()?;
        Ok(vals)
    }

    /// `ValidatorSetFromExistingValidators`: rebuilds a set without changing
    /// priorities; the proposer is the previous proposer (lowest priority).
    pub fn from_existing_validators(list: Vec<Validator>) -> Result<Self> {
        if list.is_empty() {
            return Err(Error::msg("validator set is empty"));
        }
        let mut seen: Vec<&[u8]> = Vec::with_capacity(list.len());
        for v in &list {
            v.validate_basic()
                .map_err(|e| e.wrap("can't create validator set: "))?;
            if seen.contains(&v.address.as_slice()) {
                return Err(Error::msg(format!(
                    "duplicate validator address {}",
                    hex_upper(&v.address)
                )));
            }
            seen.push(&v.address);
        }
        let mut vals = Self {
            validators: list,
            ..Self::default()
        };
        vals.check_all_keys_have_same_type();
        let mut prev = 0;
        for i in 1..vals.validators.len() {
            if vals.validators[prev].wins_over(&vals.validators[i]) {
                prev = i;
            }
        }
        vals.proposer = Some(ProposerRef::Member(vals.validators[prev].address.clone()));
        vals.update_total_voting_power()?;
        sort_by_voting_power(&mut vals.validators);
        Ok(vals)
    }
}

/// `processChanges`: sorts by address, rejects duplicates and bad powers,
/// splits into updates and removals.
fn process_changes(orig: &[Validator]) -> Result<(Vec<Validator>, Vec<Validator>)> {
    let mut changes = orig.to_vec();
    sort_by_address(&mut changes);
    let mut removals = Vec::new();
    let mut updates = Vec::new();
    // Go starts from a nil address; bytes.Equal(nil, empty) is true.
    let mut prev_addr: &[u8] = &[];
    for v in &changes {
        if v.address == prev_addr {
            return Err(Error::msg(format!(
                "duplicate entry {v} in {}",
                validator_list_go(&changes)
            )));
        }
        if v.voting_power < 0 {
            return Err(Error::msg(format!(
                "voting power can't be negative: {}",
                v.voting_power
            )));
        } else if v.voting_power > MAX_TOTAL_VOTING_POWER {
            return Err(Error::msg(format!(
                "to prevent clipping/overflow, voting power can't be higher than {MAX_TOTAL_VOTING_POWER}, got {}",
                v.voting_power
            )));
        } else if v.voting_power == 0 {
            removals.push(v.clone());
        } else {
            updates.push(v.clone());
        }
        prev_addr = &v.address;
    }
    Ok((updates, removals))
}

fn delta(update: &Validator, vals: &ValidatorSet) -> i64 {
    match vals.get_by_address(&update.address) {
        Some((_, v)) => update.voting_power - v.voting_power,
        None => update.voting_power,
    }
}

fn verify_updates(updates: &[Validator], vals: &ValidatorSet, removed_power: i64) -> Result<i64> {
    let mut deltas: Vec<i64> = updates.iter().map(|u| delta(u, vals)).collect();
    deltas.sort_unstable();
    let mut tvp_after_removals = vals.total_voting_power().wrapping_sub(removed_power);
    for d in deltas {
        tvp_after_removals = tvp_after_removals.wrapping_add(d);
        if tvp_after_removals > MAX_TOTAL_VOTING_POWER {
            return Err(Error::TotalVotingPowerOverflow);
        }
    }
    Ok(tvp_after_removals.wrapping_add(removed_power))
}

fn num_new_validators(updates: &[Validator], vals: &ValidatorSet) -> usize {
    updates
        .iter()
        .filter(|u| !vals.has_address(&u.address))
        .count()
}

fn compute_new_priorities(updates: &mut [Validator], vals: &ValidatorSet, updated_tvp: i64) {
    for u in updates {
        match vals.get_by_address(&u.address) {
            None => {
                u.proposer_priority = updated_tvp.wrapping_add(updated_tvp >> 3).wrapping_neg();
            }
            Some((_, v)) => u.proposer_priority = v.proposer_priority,
        }
    }
}

fn verify_removals(deletes: &[Validator], vals: &ValidatorSet) -> Result<i64> {
    let mut removed = 0i64;
    for d in deletes {
        match vals.get_by_address(&d.address) {
            None => {
                return Err(Error::msg(format!(
                    "failed to find validator {} to remove",
                    hex_upper(&d.address)
                )));
            }
            Some((_, v)) => removed += v.voting_power,
        }
    }
    Ok(removed)
}
