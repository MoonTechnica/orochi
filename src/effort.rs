//! The effort axis, in whatever words the agent uses for it.
//!
//! ACP hands over a `thought_level` selector whose values are the agent's own — and often the
//! provider's server's own: `codex-acp` builds its option from the app-server's
//! `supportedReasoningEfforts`, and `claude-agent-acp` from the SDK's `supportedEffortLevels`
//! plus a `default` row of its own (read from both installed adapters, 2026-09-25). So the
//! vocabulary is not Orochi's to fix, and a policy that names `high` cannot be the only way to
//! ask for more thought.
//!
//! One ladder answers both questions, because they were separately wrong and cancelled out: a
//! vocabulary the policy did not name left every complexity on the agent's default level, and
//! the cost factor fell to its middle for every level, so no test could see that the axis had
//! stopped moving. Whatever chooses a level and whatever prices it read the same rungs here.

/// Rungs, cheapest first. Four is what the cost factors already distinguished.
pub const RUNGS: u8 = 4;

/// Where a word sits on the ladder, for the words that say so. The canonical five
/// (`minimal`/`none`, `low`, `medium`, `high`, `xhigh`/`max`) are what OpenAI, Anthropic and
/// Google all use; the rest are the words other agents use for the same thing.
pub fn rank(value: &str) -> Option<u8> {
    // `x-high`, `x_high` and `very high` are one word wearing three spellings.
    let value: String = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    Some(match value.as_str() {
        "none" | "off" | "disabled" | "no" | "never" => 0,
        "minimal" | "min" | "minimum" | "lowest" | "fast" | "fastest" | "quick" | "instant"
        | "speed" | "brief" => 0,
        "low" | "light" | "lite" | "short" | "shallow" | "concise" => 1,
        "medium" | "med" | "moderate" | "balanced" | "standard" | "normal" | "default" | "auto"
        | "automatic" | "dynamic" | "recommended" => 2,
        "high" | "thorough" | "deep" | "deeper" | "extended" | "long" | "careful" | "hard"
        | "think" | "detailed" => 3,
        "xhigh" | "veryhigh" | "extrahigh" | "max" | "maximum" | "highest" | "ultra"
        | "ultrathink" | "exhaustive" | "deepest" | "thinkharder" | "thinkhardest" => 4,
        _ => return None,
    })
}

/// One agent's effort vocabulary, placed on the ladder.
///
/// Values it recognises are placed by their word. A value it does not recognise is placed
/// between its recognised neighbours in the order the agent advertised them, so a private word
/// beside two known ones lands where the agent put it. When nothing at all is recognised, the
/// advertised order is taken as ascending — that is what both installed adapters do, and an
/// ordering is the only thing left to read.
#[derive(Debug, Clone)]
pub struct Ladder {
    entries: Vec<(String, u8)>,
}

impl Ladder {
    pub fn new(values: &[String]) -> Self {
        let known: Vec<Option<u8>> = values.iter().map(|v| rank(v)).collect();
        let entries = if known.iter().all(Option::is_none) {
            spread(values)
        } else {
            interpolate(values, &known)
        };
        Self { entries }
    }
    /// What a token at this level costs against the cheapest one, for a value of this ladder or
    /// any other word. Levels a ladder cannot place, and models with no effort axis at all,
    /// are priced at the middle rung: neither a discount nor a premium nobody measured.
    pub fn factor(&self, value: &str) -> f64 {
        factor(self.placed(value).or_else(|| rank(value)))
    }
    /// The rung this ladder puts a value on.
    pub fn placed(&self, value: &str) -> Option<u8> {
        self.entries
            .iter()
            .find(|(v, _)| v == value)
            .map(|(_, rank)| *rank)
    }
    /// The values on one rung, in the order the agent advertised them.
    pub fn at(&self, rank: u8) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(move |(_, r)| *r == rank)
            .map(|(v, _)| v.as_str())
    }
    /// The values nearest a rung, closest first, a tie going to the dearer one: a rung that
    /// does not exist here was still asked for, and asking for less thought than the policy
    /// wanted is the worse way to miss.
    pub fn nearest(&self, rank: u8) -> Vec<&str> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|(_, r)| (r.abs_diff(rank), std::cmp::Reverse(*r)));
        entries.iter().map(|(v, _)| v.as_str()).collect()
    }
}

/// The cost of a rung, against the cheapest. The canonical words kept the factors they had
/// before there was a ladder, so nothing that was already priced moves.
pub fn factor(rank: Option<u8>) -> f64 {
    match rank {
        Some(0 | 1) => 1.0,
        Some(2) => 1.2,
        Some(3) => 1.6,
        Some(_) => 2.1,
        None => 1.2,
    }
}

/// Nothing recognised: the advertised order, stretched over the rungs.
fn spread(values: &[String]) -> Vec<(String, u8)> {
    let last = values.len().saturating_sub(1);
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let rank = if last == 0 {
                2
            } else {
                ((index * RUNGS as usize * 2 + last) / (last * 2)) as u8
            };
            (value.clone(), rank.min(RUNGS))
        })
        .collect()
}

/// Something recognised: the unrecognised values take the rung between their recognised
/// neighbours, or one step beyond the only neighbour they have.
fn interpolate(values: &[String], known: &[Option<u8>]) -> Vec<(String, u8)> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let rank = known[index].unwrap_or_else(|| {
                let before = known[..index].iter().rev().flatten().next().copied();
                let after = known[index + 1..].iter().flatten().next().copied();
                match (before, after) {
                    (Some(a), Some(b)) => (a + b) / 2,
                    (Some(a), None) => (a + 1).min(RUNGS),
                    (None, Some(b)) => b.saturating_sub(1),
                    (None, None) => 2,
                }
            });
            (value.clone(), rank.min(RUNGS))
        })
        .collect()
}
