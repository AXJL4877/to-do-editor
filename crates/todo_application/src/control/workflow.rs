//! Workflow / production snapshots for the shared control protocol.
//! Pure mapping from session status + placement; the desktop host still
//! owns start/stop (see `ai-workflow-control.md`).

use super::{
    PlacementSummary, ProductionFailureView, ProductionNodeProgress, ProductionSnapshot,
    WorkflowRunState,
};
use crate::productions::{ProductionStatus, StatusState};
use todo_production::Production;

pub fn ui_state_name(state: StatusState) -> &'static str {
    match state {
        StatusState::Queued => "queued",
        StatusState::Running => "running",
        StatusState::Waiting => "waiting",
        StatusState::Attention => "attention",
        StatusState::Done => "done",
        StatusState::Failed => "failed",
    }
}

/// Map panel aggregate + workflow record into the commercial run-state set.
pub fn commercial_run_state(
    status: &ProductionStatus,
    workflow_error: Option<&str>,
    has_live_run: bool,
    cancelled: bool,
) -> WorkflowRunState {
    commercial_run_state_with_placement(status, workflow_error, has_live_run, cancelled, false)
}

/// Same as [`commercial_run_state`], but demotes a fake "completed" / stuck
/// "running" when generation finished with nothing adoptable on the timeline.
pub fn commercial_run_state_with_placement(
    status: &ProductionStatus,
    workflow_error: Option<&str>,
    has_live_run: bool,
    cancelled: bool,
    placement_failed: bool,
) -> WorkflowRunState {
    if cancelled {
        return WorkflowRunState::Cancelled;
    }
    if let Some(err) = workflow_error {
        if err.contains("停止") || err.contains("cancel") {
            return WorkflowRunState::Cancelled;
        }
        return WorkflowRunState::PausedError;
    }
    if !status.failures.is_empty() || status.run_failures > 0 {
        return WorkflowRunState::PausedError;
    }
    if status.stages.iter().any(|s| s.state == StatusState::Failed)
        || status.nodes.iter().any(|n| n.state == StatusState::Failed)
    {
        return WorkflowRunState::PausedError;
    }
    if status.stages.iter().any(|s| s.state == StatusState::Waiting)
        || status.nodes.iter().any(|n| n.state == StatusState::Waiting)
    {
        return WorkflowRunState::WaitingService;
    }
    // Adopt/placement dead-end: Timeline may still look "Running" while
    // unadoptable blocks every file — that must not stay Running/Completed.
    if placement_failed && !has_live_run {
        return WorkflowRunState::PausedError;
    }
    if has_live_run
        || status.stages.iter().any(|s| s.state == StatusState::Running)
        || status.nodes.iter().any(|n| n.state == StatusState::Running)
    {
        return WorkflowRunState::Running;
    }
    if status.total > 0
        && status.done == status.total
        && status
            .stages
            .iter()
            .all(|s| s.state == StatusState::Done || s.total == 0)
    {
        return WorkflowRunState::Completed;
    }
    if status.total == 0 && !has_live_run {
        return WorkflowRunState::Idle;
    }
    WorkflowRunState::Idle
}

/// True when the run looks finished yet nothing usable sits on the timeline
/// because adopt/placement failed (not because the user cleared clips).
pub fn placement_failed_after_done(
    doc: &Production,
    deleted_on_timeline: usize,
    placement_blocked: bool,
    unadoptable: usize,
    pending_adopt: usize,
) -> bool {
    placement_failed_after_done_ex(
        doc,
        deleted_on_timeline,
        placement_blocked,
        unadoptable,
        pending_adopt,
        false,
    )
}

/// Like [`placement_failed_after_done`], plus in-memory attention / stuck-pending.
pub fn placement_failed_after_done_ex(
    doc: &Production,
    deleted_on_timeline: usize,
    placement_blocked: bool,
    unadoptable: usize,
    pending_adopt: usize,
    attention_says_failed: bool,
) -> bool {
    if !doc.placement.placed.is_empty() {
        return false;
    }
    if doc
        .placement
        .error
        .as_ref()
        .is_some_and(|e| !e.trim().is_empty())
        || attention_says_failed
        || placement_blocked
        || unadoptable > 0
    {
        return true;
    }
    // Generate finished with nothing placed: pending adopt that never clears
    // (missing files / dead workdir) must not stay "still adopting" → completed.
    if pending_adopt > 0 {
        return true;
    }
    doc.placement.tombstones.is_empty() && deleted_on_timeline == 0
}

pub fn placement_summary(
    doc: &Production,
    deleted_on_timeline: usize,
    placement_blocked: bool,
    unadoptable: usize,
    pending_adopt: usize,
) -> PlacementSummary {
    PlacementSummary {
        placed: doc.placement.placed.len(),
        tombstones: doc.placement.tombstones.len(),
        positions: doc.placement.positions.len(),
        pending_adopt,
        deleted_on_timeline,
        placement_blocked,
        unadoptable,
    }
}

pub fn snapshot_from(
    status: &ProductionStatus,
    doc: &Production,
    local_run_ids: Vec<u64>,
    has_live_run: bool,
) -> ProductionSnapshot {
    snapshot_from_with_placement(status, doc, local_run_ids, has_live_run, false, 0, 0)
}

/// Same as [`snapshot_from`], with host-side placement pause / adopt queues.
pub fn snapshot_from_with_placement(
    status: &ProductionStatus,
    doc: &Production,
    local_run_ids: Vec<u64>,
    has_live_run: bool,
    placement_blocked: bool,
    unadoptable: usize,
    pending_adopt: usize,
) -> ProductionSnapshot {
    let workflow_error = doc.workflow.as_ref().and_then(|w| w.error.clone());
    let attention_says_failed = status
        .attention
        .iter()
        .any(|line| line.starts_with("入轨未成功"));
    let placement_failed = placement_failed_after_done_ex(
        doc,
        status.deleted,
        placement_blocked,
        unadoptable,
        pending_adopt,
        attention_says_failed,
    );
    let run_state = commercial_run_state_with_placement(
        status,
        workflow_error.as_deref(),
        has_live_run,
        false,
        placement_failed,
    );
    let mut attention = status.attention.clone();
    // Collapse 80+ per-file「入轨未成功」into one readable card.
    let fail_notes: Vec<String> = attention
        .iter()
        .filter(|line| line.starts_with("入轨未成功"))
        .cloned()
        .collect();
    if fail_notes.len() > 1 {
        attention.retain(|line| !line.starts_with("入轨未成功"));
        attention.insert(
            0,
            format!(
                "入轨未成功：{} 项素材无法入轨（路径缺失或登记失败）。下一步：点「重试」，或检查工作目录。",
                fail_notes.len()
            ),
        );
    } else if placement_failed && fail_notes.is_empty() {
        let detail = if pending_adopt > 0 {
            format!("{pending_adopt} 个素材入轨卡住（文件可能已丢失）")
        } else if unadoptable > 0 {
            format!("{unadoptable} 个素材无法入轨（unadoptable）")
        } else if placement_blocked {
            "入轨被暂停（轨道锁定/重叠或上次 Apply 被拒）".into()
        } else {
            "生成已结束但时间轴上没有已入轨片段".into()
        };
        attention.insert(
            0,
            format!("入轨未成功：{detail}。下一步：点「重试」，或检查轨道是否锁定。"),
        );
    }
    // Prefer placement/retry cards over a long workdir note at the top.
    if attention.iter().any(|line| line.starts_with("入轨未成功")) {
        attention.sort_by_key(|line| {
            if line.starts_with("入轨未成功") {
                0
            } else if line.contains("工作目录已切换") {
                2
            } else {
                1
            }
        });
    }
    let mut ui_aggregate = status
        .stages
        .iter()
        .map(|s| s.state)
        .chain(status.nodes.iter().map(|n| n.state))
        .fold(StatusState::Queued, |acc, s| {
            // Prefer failed > waiting > running > attention > done > queued.
            use StatusState::*;
            let rank = |x: StatusState| match x {
                Failed => 5,
                Waiting => 4,
                Running => 3,
                Attention => 2,
                Done => 1,
                Queued => 0,
            };
            if rank(s) > rank(acc) {
                s
            } else {
                acc
            }
        });
    if run_state == WorkflowRunState::PausedError
        && matches!(
            ui_aggregate,
            StatusState::Done | StatusState::Running | StatusState::Queued | StatusState::Attention
        )
    {
        ui_aggregate = StatusState::Failed;
    }
    ProductionSnapshot {
        production_id: status.id.clone(),
        title: status.title.clone(),
        extension_id: status.extension.clone(),
        run_state,
        ui_aggregate: ui_state_name(ui_aggregate).into(),
        done: status.done,
        total: status.total,
        local_run_ids,
        nodes: status
            .nodes
            .iter()
            .map(|n| ProductionNodeProgress {
                key: n.key.clone(),
                label: n.label.clone(),
                ui_state: ui_state_name(n.state).into(),
                detail: n.detail.clone(),
            })
            .collect(),
        recent_errors: status
            .failures
            .iter()
            .map(|f| ProductionFailureView {
                key: f.key.0.clone(),
                class: f.class.clone(),
                message: f.message.clone(),
                possibly_billed: f.possibly_billed,
            })
            .collect(),
        attention,
        placement: placement_summary(
            doc,
            status.deleted,
            placement_blocked,
            unadoptable,
            pending_adopt,
        ),
        workflow_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::productions::{ProductionStatus, StageKind, StageStatus, StatusState};

    fn status_with(stage: StatusState) -> ProductionStatus {
        ProductionStatus {
            id: "p".into(),
            title: "t".into(),
            extension: "ext".into(),
            script: String::new(),
            done: 0,
            total: 1,
            deleted: 0,
            stages: vec![StageStatus {
                kind: StageKind::Generate,
                done: 0,
                total: 1,
                state: stage,
                phase: None,
            }],
            nodes: vec![],
            failures: vec![],
            run_failures: 0,
            attention: vec![],
        }
    }

    #[test]
    fn commercial_states_cover_the_panel_set() {
        assert_eq!(
            commercial_run_state(&status_with(StatusState::Queued), None, false, false),
            WorkflowRunState::Idle
        );
        assert_eq!(
            commercial_run_state(&status_with(StatusState::Running), None, true, false),
            WorkflowRunState::Running
        );
        assert_eq!(
            commercial_run_state(&status_with(StatusState::Waiting), None, false, false),
            WorkflowRunState::WaitingService
        );
        assert_eq!(
            commercial_run_state(
                &status_with(StatusState::Failed),
                Some("timeout"),
                false,
                false
            ),
            WorkflowRunState::PausedError
        );
        assert_eq!(
            commercial_run_state(&status_with(StatusState::Running), None, true, true),
            WorkflowRunState::Cancelled
        );
        let mut done = status_with(StatusState::Done);
        done.done = 1;
        done.total = 1;
        assert_eq!(
            commercial_run_state(&done, None, false, false),
            WorkflowRunState::Completed
        );
        assert_eq!(
            commercial_run_state_with_placement(&done, None, false, false, true),
            WorkflowRunState::PausedError
        );
    }

    #[test]
    fn fake_completed_without_placement_is_paused_error() {
        let mut done = status_with(StatusState::Done);
        done.done = 1;
        done.total = 1;
        let mut doc = Production::new(
            "p",
            todo_production::PackRef {
                id: "ext".into(),
                version: "1".into(),
            },
            "t",
            Default::default(),
            todo_production::SplitMode::Paragraph,
            todo_production::segment::Limits {
                target: 120,
                max: 200,
            },
        );
        // Nothing placed, nothing deleted by user → placement failed.
        assert!(placement_failed_after_done(&doc, 0, false, 0, 0));
        assert!(placement_failed_after_done(&doc, 0, false, 3, 0));
        assert!(placement_failed_after_done(&doc, 0, true, 0, 0));
        // User cleared the timeline: tombstones / deleted, no unadoptable → not a fail.
        doc.placement
            .tombstones
            .insert(("k".into(), "r".into()));
        assert!(!placement_failed_after_done(&doc, 1, false, 0, 0));
        // Unadoptable pile still fails even if user also deleted.
        assert!(placement_failed_after_done(&doc, 1, false, 3, 0));
        // Stuck pending adopt with nothing placed → failed (not "still adopting").
        assert!(placement_failed_after_done_ex(
            &doc, 1, false, 0, 82, false
        ));
        assert!(placement_failed_after_done_ex(
            &doc, 1, false, 0, 82, true
        ));
        // Persisted placement.error demotes without in-memory unadoptable.
        doc.placement.error = Some("素材登记失败".into());
        assert!(placement_failed_after_done(&doc, 1, false, 0, 0));

        let snap = snapshot_from_with_placement(&done, &doc, vec![], false, false, 0, 82);
        assert_eq!(snap.run_state, WorkflowRunState::PausedError);
        assert!(
            snap.attention.iter().any(|a| a.starts_with("入轨未成功")),
            "attention={:?}",
            snap.attention
        );
        assert_eq!(snap.ui_aggregate, "failed");
    }

    #[test]
    fn stuck_timeline_running_with_unadoptable_is_not_completed_or_running() {
        // Generate done, Timeline still "Running" because files never adopted.
        let status = ProductionStatus {
            id: "p".into(),
            title: "t".into(),
            extension: "ext".into(),
            script: String::new(),
            done: 1,
            total: 1,
            deleted: 0,
            stages: vec![
                StageStatus {
                    kind: StageKind::Generate,
                    done: 1,
                    total: 1,
                    state: StatusState::Done,
                    phase: None,
                },
                StageStatus {
                    kind: StageKind::Timeline,
                    done: 0,
                    total: 3,
                    state: StatusState::Running,
                    phase: None,
                },
            ],
            nodes: vec![],
            failures: vec![],
            run_failures: 0,
            attention: vec![],
        };
        let doc = Production::new(
            "p",
            todo_production::PackRef {
                id: "ext".into(),
                version: "1".into(),
            },
            "t",
            Default::default(),
            todo_production::SplitMode::Paragraph,
            todo_production::segment::Limits {
                target: 120,
                max: 200,
            },
        );
        // Without placement_failed flag, old logic would report Running forever.
        assert_eq!(
            commercial_run_state(&status, None, false, false),
            WorkflowRunState::Running
        );
        let snap = snapshot_from_with_placement(&status, &doc, vec![], false, false, 83, 0);
        assert_eq!(
            snap.run_state,
            WorkflowRunState::PausedError,
            "unadoptable pile must not stay running/completed"
        );
        assert_ne!(snap.run_state, WorkflowRunState::Completed);
        assert!(
            snap.attention.iter().any(|a| a.starts_with("入轨未成功")),
            "attention={:?}",
            snap.attention
        );
        assert_eq!(snap.ui_aggregate, "failed");
        // Live graph still running wins over placement pause.
        let live = snapshot_from_with_placement(&status, &doc, vec![1], true, false, 83, 0);
        assert_eq!(live.run_state, WorkflowRunState::Running);
    }
}
