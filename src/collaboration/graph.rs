//! The order of the work: which parts may run at the same time and which must wait.
//!
//! A model may propose the parts; everything here is Orochi's own deterministic reading of
//! them. Running side by side needs evidence — separate, declared write sets and no stated
//! dependency — and any doubt puts one part after the other.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

pub const MAX_PARTS: usize = 12;
const MAX_BRIEF: usize = 2048;
const MAX_ID: usize = 32;

/// One piece of the work, as the plan (or the design turn) states it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: String,
    pub brief: String,
    /// Relative paths this part writes. Empty means it may write anywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Parts whose code this one needs before it can start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
}

/// What one session carries out: a part, or a straight chain of parts fused into one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unit {
    /// The first part of the chain; names the session and its workspace.
    pub id: String,
    /// Every part this unit carries out, in order.
    pub parts: Vec<String>,
    pub brief: String,
    /// Empty when any of its parts may write anywhere.
    pub paths: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Two parts may touch the same file: the same path, one inside the other, or either one
/// never said where it writes.
fn overlap(a: &[String], b: &[String]) -> bool {
    a.is_empty()
        || b.is_empty()
        || a.iter().any(|x| {
            b.iter()
                .any(|y| Path::new(x).starts_with(y) || Path::new(y).starts_with(x))
        })
}

fn closure(edges: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let mut reach = edges.to_vec();
    let n = reach.len();
    for k in 0..n {
        let through = reach[k].clone();
        for row in reach.iter_mut().filter(|row| row[k]) {
            for (cell, onward) in row.iter_mut().zip(&through) {
                *cell |= *onward;
            }
        }
    }
    reach
}

/// Edges between groups of parts, without the ones another path already implies.
fn reduced(nodes: &[Vec<usize>], before: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let n = nodes.len();
    let mut edges = vec![vec![false; n]; n];
    for (a, from) in nodes.iter().enumerate() {
        for (b, to) in nodes.iter().enumerate() {
            edges[a][b] = a != b && from.iter().any(|i| to.iter().any(|j| before[*i][*j]));
        }
    }
    let reach = closure(&edges);
    let mut direct = edges.clone();
    for a in 0..n {
        for b in 0..n {
            if edges[a][b] && (0..n).any(|k| k != a && k != b && reach[a][k] && reach[k][b]) {
                direct[a][b] = false;
            }
        }
    }
    direct
}

/// Fuses a group into its only successor while that successor has no other predecessor.
fn contract(nodes: &mut Vec<Vec<usize>>, before: &[Vec<bool>]) {
    loop {
        let edges = reduced(nodes, before);
        let n = nodes.len();
        let chain = (0..n).find_map(|a| {
            let next: Vec<usize> = (0..n).filter(|b| edges[a][*b]).collect();
            (next.len() == 1 && (0..n).filter(|p| edges[*p][next[0]]).count() == 1)
                .then(|| (a, next[0]))
        });
        let Some((a, b)) = chain else { return };
        let tail = nodes.remove(b);
        let a = if b < a { a - 1 } else { a };
        nodes[a].extend(tail);
        nodes.sort_by_key(|members| members.iter().min().copied());
    }
}

/// Longest-path layers, each in listed order.
fn layers(nodes: &[Vec<usize>], before: &[Vec<bool>]) -> Vec<Vec<usize>> {
    let edges = reduced(nodes, before);
    let n = nodes.len();
    let mut depth = vec![0usize; n];
    // n rounds settle every longest path of an acyclic graph.
    for _ in 0..n {
        for a in 0..n {
            for b in 0..n {
                if edges[a][b] {
                    depth[b] = depth[b].max(depth[a] + 1);
                }
            }
        }
    }
    let height = depth.iter().max().map_or(0, |d| d + 1);
    (0..height)
        .map(|level| (0..n).filter(|node| depth[*node] == level).collect())
        .collect()
}

/// The waves a list of parts runs in when at most `width` sessions may work at once.
/// Refuses the whole list rather than trimming it into something nobody proposed.
pub fn waves(parts: &[Part], width: usize) -> Result<Vec<Vec<Unit>>> {
    ensure!(
        (1..=MAX_PARTS).contains(&parts.len()),
        "a work graph has 1..={MAX_PARTS} parts"
    );
    ensure!(width >= 1, "a work graph needs at least one seat");
    let mut ids = BTreeSet::new();
    for part in parts {
        ensure!(
            valid_id(&part.id) && ids.insert(part.id.as_str()),
            "part IDs are unique lowercase letters, digits and '-', at most {MAX_ID} characters"
        );
        ensure!(
            !part.brief.trim().is_empty() && part.brief.len() <= MAX_BRIEF,
            "part {} needs a brief of at most {MAX_BRIEF} bytes",
            part.id
        );
        ensure!(
            part.paths.len() <= 32 && part.paths.iter().all(|path| super::relative(path)),
            "part {} may name at most 32 relative paths",
            part.id
        );
        ensure!(
            part.after.len() <= MAX_PARTS,
            "part {} lists too many dependencies",
            part.id
        );
    }
    let n = parts.len();
    let mut before = vec![vec![false; n]; n];
    for (j, part) in parts.iter().enumerate() {
        for dependency in &part.after {
            let i = parts
                .iter()
                .position(|p| p.id == *dependency)
                .with_context(|| format!("part {} waits for an unknown part", part.id))?;
            ensure!(i != j, "part {} waits for itself", part.id);
            before[i][j] = true;
        }
    }
    let reach = closure(&before);
    ensure!(
        (0..n).all(|i| !reach[i][i]),
        "the parts wait for each other in a cycle"
    );
    // Neither order was stated, but both may write the same place: the one listed first goes
    // first. Ordering an unordered pair can never close a cycle.
    for i in 0..n {
        for j in i + 1..n {
            let reach = closure(&before);
            if !reach[i][j] && !reach[j][i] && overlap(&parts[i].paths, &parts[j].paths) {
                before[i][j] = true;
            }
        }
    }
    let mut nodes: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let levels = loop {
        contract(&mut nodes, &before);
        let levels = layers(&nodes, &before);
        let mut widened = false;
        for level in levels.iter().filter(|level| level.len() > width) {
            // A wave too wide for the seats runs as consecutive waves, in listed order; the
            // added order may let a chain fuse on the next pass.
            let chunks: Vec<&[usize]> = level.chunks(width).collect();
            for pair in chunks.windows(2) {
                for a in pair[0].iter().flat_map(|node| &nodes[*node]) {
                    for b in pair[1].iter().flat_map(|node| &nodes[*node]) {
                        before[*a][*b] = true;
                    }
                }
            }
            widened = true;
        }
        if !widened {
            break levels;
        }
    };
    Ok(levels
        .into_iter()
        .map(|level| {
            level
                .into_iter()
                .map(|node| unit(parts, &nodes[node]))
                .collect()
        })
        .collect())
}

fn unit(parts: &[Part], members: &[usize]) -> Unit {
    let chain: Vec<&Part> = members.iter().map(|i| &parts[*i]).collect();
    let brief = if chain.len() == 1 {
        chain[0].brief.clone()
    } else {
        chain
            .iter()
            .map(|p| format!("{}: {}", p.id, p.brief))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let paths = if chain.iter().any(|p| p.paths.is_empty()) {
        vec![]
    } else {
        let mut seen = BTreeSet::new();
        chain
            .iter()
            .flat_map(|p| &p.paths)
            .filter(|path| seen.insert(path.as_str()))
            .cloned()
            .collect()
    };
    Unit {
        id: chain[0].id.clone(),
        parts: chain.iter().map(|p| p.id.clone()).collect(),
        brief,
        paths,
    }
}

/// The order in one line: `schema+api ‖ web → join`.
pub fn summary(waves: &[Vec<Unit>]) -> String {
    waves
        .iter()
        .map(|wave| {
            wave.iter()
                .map(|unit| unit.parts.join("+"))
                .collect::<Vec<_>>()
                .join(" ‖ ")
        })
        .collect::<Vec<_>>()
        .join(" → ")
}
