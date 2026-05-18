//! Subroot path calculation compatible with Go `GetSubrootPaths`.

use crate::{Error, Result};

/// Path from a row root to a covered subroot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubrootPath {
    /// Zero-based row index in the square.
    pub row: usize,
    /// Branch path from row root to subroot. `false` means left, `true` means right.
    pub path: Vec<bool>,
}

/// Return row-grouped subroot paths for a share range.
pub fn subroot_paths(
    square_size: usize,
    share_start: usize,
    share_count: usize,
) -> Result<Vec<Vec<SubrootPath>>> {
    let raw_paths = raw_subroot_paths(square_size, share_start, share_count)?;
    let start_row = share_start / square_size;
    Ok(raw_paths
        .into_iter()
        .enumerate()
        .map(|(row_offset, paths)| {
            paths
                .into_iter()
                .map(|path| SubrootPath {
                    row: start_row + row_offset,
                    path,
                })
                .collect()
        })
        .collect())
}

fn raw_subroot_paths(
    square_size: usize,
    share_start: usize,
    share_count: usize,
) -> Result<Vec<Vec<Vec<bool>>>> {
    if square_size < 2 || !square_size.is_power_of_two() {
        return Err(Error::InvalidSubrootPath);
    }
    if share_count == 0 {
        return Err(Error::InvalidSubrootPath);
    }

    let idx_end = share_start
        .checked_add(share_count)
        .ok_or(Error::InvalidSubrootPath)?;
    let shares = square_size
        .checked_mul(square_size)
        .ok_or(Error::InvalidSubrootPath)?;
    if share_start >= shares || idx_end > shares {
        return Err(Error::InvalidSubrootPath);
    }

    let start_row = share_start / square_size;
    let last_index = idx_end - 1;
    let mut closing_row = last_index / square_size;
    if last_index % square_size != 0 {
        closing_row += 1;
    }

    let share_start_in_row = share_start % square_size;
    let share_end = last_index % square_size;

    if share_count == 1 {
        return Ok(vec![vec![subdivide(share_start_in_row, square_size)]]);
    }

    if start_row == closing_row.saturating_sub(1) {
        return Ok(vec![prune(share_start_in_row, share_end, square_size)]);
    }

    let left = raw_subroot_paths(
        square_size,
        share_start_in_row,
        square_size - share_start_in_row,
    )?;
    let right = raw_subroot_paths(square_size, 0, share_end + 1)?;

    let mut top = Vec::new();
    top.push(left[0].clone());
    for _ in 1..(closing_row - start_row).saturating_sub(1) {
        top.push(vec![Vec::new()]);
    }
    top.push(right[0].clone());
    Ok(top)
}

fn subdivide(idx_start: usize, width: usize) -> Vec<bool> {
    let path_len = usize::BITS - width.leading_zeros() - 1;
    let mut path = Vec::with_capacity(path_len as usize);
    for bit in (0..path_len).rev() {
        path.push((idx_start & (1usize << bit)) != 0);
    }
    path
}

fn extract_branch(path: &[bool], index: usize, offset: usize, branch: bool) -> Vec<bool> {
    let mut capture = path.to_vec();
    capture[index] = branch;
    capture.truncate(index + offset);
    capture
}

fn prune(mut idx_start: usize, mut idx_end: usize, max_width: usize) -> Vec<Vec<bool>> {
    if idx_end == 0 || max_width == 0 || idx_start > idx_end || idx_end >= max_width {
        return Vec::new();
    }

    let mut path_start = subdivide(idx_start, max_width);
    let path_end = subdivide(idx_end, max_width);

    if idx_start + 1 >= idx_end {
        if idx_start % 2 == 1 {
            return vec![path_start, path_end];
        }
        path_start.pop();
        return vec![path_start];
    }

    let mut pruned_paths = Vec::new();
    let mut preprocessed_paths = Vec::new();

    if idx_start % 2 == 1 {
        idx_start += 1;
        preprocessed_paths.push(path_start);
        path_start = subdivide(idx_start, max_width);
    }

    if idx_end % 2 == 0 {
        idx_end -= 1;
        preprocessed_paths.push(path_end);
    }

    let tree_depth = path_start.len();
    let mut captured_span = 0usize;
    let mut right_traversed = false;

    for index in (0..tree_depth).rev() {
        if captured_span >= idx_end {
            break;
        }

        let node_span = 1usize << (tree_depth - index);
        if path_start[index] {
            idx_start -= node_span / 2;
            right_traversed = true;
        } else {
            let last_node = node_span + idx_start - 1;
            if last_node <= idx_end {
                captured_span = last_node;
                if right_traversed {
                    pruned_paths.push(extract_branch(&path_start, index, 1, true));
                } else {
                    pruned_paths = vec![path_start[..index].to_vec()];
                }
            } else {
                break;
            }
        }
    }

    let mut combined = preprocessed_paths;
    combined.extend(pruned_paths);
    if captured_span == idx_end {
        return combined;
    }
    combined.extend(prune(captured_span + 1, idx_end, max_width));
    combined
}
