//! Deterministic merges of concurrent implementations and guarded application of a
//! verified result to the user's working tree.
use super::{
    Application, ApplyStatus, AttemptStatus, Cx, MergeRecord, Report, Role, TurnKind, turn,
    workspace::{self, Change},
};
use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeMap, path::Path};

fn recreate(output: &Path, name: &str) -> Result<std::path::PathBuf> {
    let path = output.join(name);
    if path.symlink_metadata().is_ok() {
        ensure!(
            !path.is_symlink() && path.canonicalize()?.starts_with(output),
            "{name} escaped collaboration output"
        );
        std::fs::remove_dir_all(&path)?;
    }
    Ok(path)
}

pub(super) fn merge_implementers(cx: &Cx<'_>, report: &mut Report, stage: usize) -> Result<()> {
    let output = cx.output;
    if let Some(last) = report.merges.last()
        && !report.sessions.iter().any(|s| {
            s.role == Role::Implementer
                && s.status == super::AttemptStatus::Completed
                && s.stage > last.stage
        })
    {
        return Ok(());
    }
    let baseline = output.join("baseline");
    let merged = recreate(output, "merged")?;
    workspace::copy_workspace(&baseline, &merged)?;
    let mut sources: Vec<String> = vec![];
    let mut conflicts = vec![];
    for index in report.plan.indices(Role::Implementer) {
        let id = report.plan.participants[index].id.clone();
        let label = if sources.is_empty() {
            "baseline".to_owned()
        } else {
            sources.join("+")
        };
        let merge = workspace::three_way(
            &baseline,
            &merged,
            &output.join(&id),
            [&label, "baseline", &id],
        )?;
        let changes: Vec<&Change> = merge.changes.iter().chain(&merge.marked).collect();
        workspace::commit(&merged, &changes)?;
        conflicts.extend(merge.conflicts);
        sources.push(id);
    }
    cx.say(format!(
        "Merged {} implementations; {} conflict(s)",
        sources.len(),
        conflicts.len()
    ));
    report.merges.push(MergeRecord {
        stage,
        sources,
        conflicts,
        wave: None,
        strays: BTreeMap::new(),
    });
    Ok(())
}

/// Merges the units of one wave onto the tree they started from. A conflict is settled here,
/// before the next wave copies the result, unless this is the last wave: the integrator
/// resolves those as part of integrating, as with parallel implementers.
pub(super) async fn join(
    cx: &Cx<'_>,
    report: &mut Report,
    stage: usize,
    wave: usize,
) -> Result<()> {
    let output = cx.output;
    let waves = report.plan.waves().context("the plan has no work graph")?;
    let base = super::wave_base(output, &waves, wave);
    let merged = super::wave_dir(output, wave).join("_merged");
    if !report.merges.iter().any(|m| m.stage == stage) {
        if merged.symlink_metadata().is_ok() {
            ensure!(
                !merged.is_symlink() && merged.canonicalize()?.starts_with(output),
                "a merged wave escaped collaboration output"
            );
            std::fs::remove_dir_all(&merged)?;
        }
        workspace::copy_workspace(&base, &merged)?;
        let mut sources: Vec<String> = vec![];
        let mut conflicts = vec![];
        let mut strays = BTreeMap::new();
        for unit in &waves[wave] {
            let theirs = super::wave_dir(output, wave).join(&unit.id);
            let label = if sources.is_empty() {
                "base".to_owned()
            } else {
                sources.join("+")
            };
            let merge = workspace::three_way(&base, &merged, &theirs, [&label, "base", &unit.id])?;
            let changes: Vec<&Change> = merge.changes.iter().chain(&merge.marked).collect();
            workspace::commit(&merged, &changes)?;
            conflicts.extend(merge.conflicts);
            let outside = workspace::changed(&base, &theirs)?
                .into_iter()
                .filter(|path| {
                    !unit.paths.is_empty() && !unit.paths.iter().any(|own| path.starts_with(own))
                })
                .count();
            strays.insert(unit.id.clone(), outside);
            sources.push(unit.id.clone());
        }
        cx.say(format!(
            "Merged {} parts of wave {}; {} conflict(s)",
            sources.len(),
            wave + 1,
            conflicts.len()
        ));
        report.merges.push(MergeRecord {
            stage,
            sources,
            conflicts,
            wave: Some(wave),
            strays,
        });
        super::save(output, report)?;
    }
    let conflicts = report
        .merges
        .iter()
        .rev()
        .find(|m| m.stage == stage)
        .map(|m| m.conflicts.clone())
        .unwrap_or_default();
    if conflicts.is_empty() || wave + 1 == waves.len() {
        return Ok(());
    }
    let resolved = report.sessions.iter().any(|s| {
        s.stage == stage && s.turn == TurnKind::Resolution && s.status == AttemptStatus::Completed
    });
    if !resolved {
        let turn = turn::untangle(report, stage, merged, &conflicts);
        turn::run_group(cx, report, stage, vec![turn]).await?;
    }
    Ok(())
}

pub(super) async fn apply(cx: &Cx<'_>, report: &mut Report, stage: usize) -> Result<()> {
    let output = cx.output;
    let verified = report
        .integration()
        .is_some_and(|s| s.outcome == crate::types::Outcome::Success);
    let application = report.application.get_or_insert(Application {
        status: ApplyStatus::Pending,
        files: vec![],
        conflicts: vec![],
        detail: None,
        rounds: 0,
    });
    if matches!(
        application.status,
        ApplyStatus::Applied | ApplyStatus::Skipped
    ) {
        return Ok(());
    }
    // Only a result whose checks actually passed may reach the user's working tree.
    if !verified {
        application.status = ApplyStatus::Skipped;
        application.detail =
            Some("not applied: the integrated result has no passing verification checks".into());
        return Ok(());
    }
    let root = report.root.clone();
    let repository = cx.store.repository_id(&root)?;
    let _lock = crate::storage::workspace_lock(cx.store.data_dir(), &repository, false)?;
    let integrated = report
        .final_workspace
        .clone()
        .context("missing integrated workspace")?;
    for _ in 0..4 {
        let status = report.application.as_ref().unwrap().status;
        match status {
            ApplyStatus::Resolving => {
                let conflicts = report.application.as_ref().unwrap().conflicts.clone();
                let resolve = output.join("resolve");
                ensure!(resolve.is_dir(), "missing conflict resolution workspace");
                let resolved = report
                    .sessions
                    .iter()
                    .filter(|s| {
                        s.stage == stage
                            && s.turn == TurnKind::Resolution
                            && s.status == super::AttemptStatus::Completed
                    })
                    .count();
                if resolved < report.application.as_ref().unwrap().rounds {
                    let turn = turn::resolution(report, stage, resolve, &conflicts, &integrated);
                    turn::run_group(cx, report, stage, vec![turn]).await?;
                }
                report.application.as_mut().unwrap().status = ApplyStatus::Resolved;
                super::save(output, report)?;
            }
            ApplyStatus::Pending | ApplyStatus::Resolved => {
                let resolved = status == ApplyStatus::Resolved;
                let (base, source) = if resolved {
                    (output.join("resolve-base"), output.join("resolve"))
                } else {
                    (output.join("baseline"), integrated.clone())
                };
                let merge = workspace::three_way(
                    &base,
                    &root,
                    &source,
                    ["working-tree", "base", "collaboration"],
                )?;
                let application = report.application.as_mut().unwrap();
                if merge.conflicts.is_empty() {
                    let changes: Vec<&Change> = merge.changes.iter().collect();
                    application.files = workspace::commit(&root, &changes)?;
                    application.status = ApplyStatus::Applied;
                    application.detail = None;
                    cx.say(format!(
                        "Applied {} file(s) to the working tree",
                        application.files.len()
                    ));
                    return Ok(());
                }
                application.conflicts = merge.conflicts;
                if resolved {
                    application.status = ApplyStatus::Pending;
                    super::save(output, report)?;
                    bail!(
                        "the working tree changed while conflicts were being resolved; resume to merge again"
                    );
                }
                // Resolve against a frozen copy of the current working tree.
                let snapshot = recreate(output, "resolve-base")?;
                let resolve = recreate(output, "resolve")?;
                workspace::copy_workspace(&root, &snapshot)?;
                workspace::copy_workspace(&snapshot, &resolve)?;
                let merge = workspace::three_way(
                    &base,
                    &snapshot,
                    &source,
                    ["working-tree", "base", "collaboration"],
                )?;
                let changes: Vec<&Change> = merge.changes.iter().chain(&merge.marked).collect();
                workspace::commit(&resolve, &changes)?;
                let application = report.application.as_mut().unwrap();
                if merge.conflicts.is_empty() {
                    // The working tree changed between the two merges; apply the fresh copy.
                    application.status = ApplyStatus::Resolved;
                } else {
                    cx.say(format!(
                        "{} conflict(s) with the working tree; starting a resolution session",
                        merge.conflicts.len()
                    ));
                    application.conflicts = merge.conflicts;
                    application.status = ApplyStatus::Resolving;
                    application.rounds += 1;
                }
                super::save(output, report)?;
            }
            ApplyStatus::Applied | ApplyStatus::Skipped => return Ok(()),
        }
    }
    bail!("working tree application did not converge")
}
