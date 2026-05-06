//! Pure decision function for the Claude Stop-hook.
//!
//! Direct port of `src/electron/stop-hook-pipeline.ts`. Lives outside
//! the runtime god-object so each branch can be unit-tested with
//! fixture snapshots, and so the decision and the side effects (e.g.
//! recording the audit signature) stay separate.
//!
//! Pipeline runs in priority order:
//!   1. Awaiting-user gate
//!   2. Q&A turn (no qualifying tool activity) — allow stop
//!   3. Subagent in flight — suppress
//!   4. Filed-but-didn't-ship advisory
//!   5. Stale-epic-children advisory
//!   6. In-progress audit (with signature dedup)
//!   7. Allow stop

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{Thread, ThreadStatus, WorkItem, WorkItemKind, WorkItemStatus};

#[derive(Debug, Clone, Default)]
pub struct ThreadSnapshot<'a> {
    pub thread: Option<&'a Thread>,
    pub work_items: &'a [WorkItem],
    /// Signature of the in_progress set the runtime last emitted an
    /// audit directive for on this thread.
    pub last_in_progress_audit_signature: Option<&'a str>,
    pub subagent_in_flight: bool,
    /// `None` is treated as "unknown — don't suppress".
    pub turn_had_activity: Option<bool>,
    pub awaiting_user: bool,
    pub turn_had_writes: bool,
    pub turn_had_filing: bool,
    pub turn_filed_ready_item: bool,
    pub filed_but_didnt_ship_fired: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StopDirective {
    pub decision: &'static str, // always "block"
    pub reason: String,
}

impl StopDirective {
    pub fn block(reason: String) -> Self {
        Self {
            decision: "block",
            reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopHookSideEffect {
    RecordAuditSignature(String),
    RecordFiledButDidntShipFired,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StopHookOutcome {
    pub directive: Option<StopDirective>,
    pub side_effects: Vec<StopHookSideEffect>,
}

/// Optional builders that turn a list of items into the human-readable
/// reason text. Each is `Option` so callers can opt out of a branch
/// (the older runtime tests used this for selective coverage).
pub struct DirectiveBuilders<'a> {
    pub build_in_progress_audit_reason: Option<&'a dyn Fn(&[WorkItem]) -> String>,
    pub build_filed_but_didnt_ship_reason: Option<&'a dyn Fn() -> String>,
    pub build_stale_epic_children_reason: Option<&'a dyn Fn(&[StaleEpicPair]) -> String>,
}

impl<'a> Default for DirectiveBuilders<'a> {
    fn default() -> Self {
        Self {
            build_in_progress_audit_reason: None,
            build_filed_but_didnt_ship_reason: None,
            build_stale_epic_children_reason: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StaleEpicPair {
    pub epic: WorkItem,
    pub stale_children: Vec<WorkItem>,
}

pub fn decide_stop_directive(
    snapshot: ThreadSnapshot<'_>,
    builders: DirectiveBuilders<'_>,
) -> StopHookOutcome {
    let mut outcome = StopHookOutcome::default();

    // 1. Awaiting-user gate.
    if snapshot.awaiting_user {
        return outcome;
    }

    // 2. Q&A turn — explicitly false means no qualifying activity.
    if snapshot.turn_had_activity == Some(false) {
        return outcome;
    }

    // Need an active (writer) thread for the rest to apply.
    let Some(thread) = snapshot.thread else {
        return outcome;
    };
    if thread.status != ThreadStatus::Active {
        return outcome;
    }

    // 3. Subagent-in-flight carve-out.
    if snapshot.subagent_in_flight {
        return outcome;
    }

    let in_progress: Vec<&WorkItem> = snapshot
        .work_items
        .iter()
        .filter(|item| item.status == WorkItemStatus::InProgress)
        .collect();

    // 4. Filed-but-didn't-ship advisory.
    if snapshot.turn_filed_ready_item
        && !snapshot.turn_had_writes
        && in_progress.is_empty()
        && !snapshot.filed_but_didnt_ship_fired
    {
        if let Some(build) = builders.build_filed_but_didnt_ship_reason {
            outcome.directive = Some(StopDirective::block(build()));
            outcome
                .side_effects
                .push(StopHookSideEffect::RecordFiledButDidntShipFired);
            return outcome;
        }
    }

    // 5. Stale-epic-children advisory.
    let stale = find_stale_epic_children_pairs(snapshot.work_items);
    if !stale.is_empty() {
        if let Some(build) = builders.build_stale_epic_children_reason {
            outcome.directive = Some(StopDirective::block(build(&stale)));
            return outcome;
        }
    }

    // 6. In-progress audit branch.
    if !in_progress.is_empty() {
        if let Some(build) = builders.build_in_progress_audit_reason {
            let in_progress_owned: Vec<WorkItem> =
                in_progress.iter().map(|i| (*i).clone()).collect();
            let signature = compute_audit_signature(&in_progress_owned);
            if snapshot.last_in_progress_audit_signature == Some(signature.as_str()) {
                // Nothing changed; suppress.
                return outcome;
            }
            outcome.directive = Some(StopDirective::block(build(&in_progress_owned)));
            outcome
                .side_effects
                .push(StopHookSideEffect::RecordAuditSignature(signature));
            return outcome;
        }
    }

    // 7. Allow stop.
    outcome
}

/// Find epics in `done`/`blocked` whose children are still
/// `ready`/`in_progress`. Pure helper exposed because the UI surfaces
/// the same data in banners.
pub fn find_stale_epic_children_pairs(items: &[WorkItem]) -> Vec<StaleEpicPair> {
    let mut pairs = Vec::new();
    for epic in items {
        if epic.kind != WorkItemKind::Epic {
            continue;
        }
        if !matches!(epic.status, WorkItemStatus::Done | WorkItemStatus::Blocked) {
            continue;
        }
        let stale_children: Vec<WorkItem> = items
            .iter()
            .filter(|child| {
                child.parent_id.as_ref() == Some(&epic.id)
                    && matches!(
                        child.status,
                        WorkItemStatus::Ready | WorkItemStatus::InProgress
                    )
            })
            .cloned()
            .collect();
        if !stale_children.is_empty() {
            pairs.push(StaleEpicPair {
                epic: epic.clone(),
                stale_children,
            });
        }
    }
    pairs
}

/// Per-thread fingerprint of the in_progress set used to detect
/// "nothing changed since last audit fire" and skip a duplicate.
pub fn compute_audit_signature(items: &[WorkItem]) -> String {
    let mut entries: Vec<String> = items
        .iter()
        .map(|item| {
            let updated_at = serde_json::to_string(&item.updated_at)
                .unwrap()
                .trim_matches('"')
                .to_string();
            format!("{}|{}|{}", item.id, updated_at, item.note_count)
        })
        .collect();
    entries.sort();
    entries.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{
        StreamId, ThreadId, Timestamp, WorkItemActorKind, WorkItemAuthor, WorkItemId,
        WorkItemPriority,
    };

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    fn active_thread() -> Thread {
        Thread {
            id: ThreadId::from("b-1"),
            stream_id: StreamId::from("s-1"),
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
        }
    }

    fn item(
        id: &str,
        kind: WorkItemKind,
        status: WorkItemStatus,
        parent: Option<&str>,
    ) -> WorkItem {
        WorkItem {
            id: WorkItemId::from(id),
            thread_id: None,
            parent_id: parent.map(WorkItemId::from),
            kind,
            title: id.into(),
            description: String::new(),
            acceptance_criteria: None,
            status,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        }
    }

    #[test]
    fn awaiting_user_suppresses_everything() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            awaiting_user: true,
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn qa_turn_allows_stop() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_had_activity: Some(false),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn non_active_thread_allows_stop() {
        let mut t = active_thread();
        t.status = ThreadStatus::Queued;
        let snap = ThreadSnapshot {
            thread: Some(&t),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn subagent_in_flight_suppresses_audit() {
        let t = active_thread();
        let items = vec![item(
            "wi-1",
            WorkItemKind::Task,
            WorkItemStatus::InProgress,
            None,
        )];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            work_items: &items,
            subagent_in_flight: true,
            ..Default::default()
        };
        let build_audit = |_: &[WorkItem]| "audit".to_string();
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn in_progress_items_trigger_audit() {
        let t = active_thread();
        let items = vec![item(
            "wi-1",
            WorkItemKind::Task,
            WorkItemStatus::InProgress,
            None,
        )];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            work_items: &items,
            ..Default::default()
        };
        let build_audit = |items: &[WorkItem]| format!("audit {} items", items.len());
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("audit directive");
        assert!(dir.reason.contains("audit 1 items"));
        assert!(matches!(
            outcome.side_effects.first(),
            Some(StopHookSideEffect::RecordAuditSignature(_))
        ));
    }

    #[test]
    fn audit_signature_dedup() {
        let t = active_thread();
        let items = vec![item(
            "wi-1",
            WorkItemKind::Task,
            WorkItemStatus::InProgress,
            None,
        )];
        let signature = compute_audit_signature(&items);
        let snap = ThreadSnapshot {
            thread: Some(&t),
            work_items: &items,
            last_in_progress_audit_signature: Some(&signature),
            ..Default::default()
        };
        let build_audit = |_: &[WorkItem]| "audit".to_string();
        let builders = DirectiveBuilders {
            build_in_progress_audit_reason: Some(&build_audit),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(
            outcome.directive.is_none(),
            "identical signature must suppress the audit"
        );
    }

    #[test]
    fn filed_but_didnt_ship_branch() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_filed_ready_item: true,
            turn_had_writes: false,
            filed_but_didnt_ship_fired: false,
            ..Default::default()
        };
        let build_filed = || "you filed but didn't ship".to_string();
        let builders = DirectiveBuilders {
            build_filed_but_didnt_ship_reason: Some(&build_filed),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("filed-but-didnt-ship directive");
        assert!(dir.reason.contains("filed but didn't ship"));
        assert!(matches!(
            outcome.side_effects.first(),
            Some(StopHookSideEffect::RecordFiledButDidntShipFired)
        ));
    }

    #[test]
    fn filed_but_didnt_ship_already_fired_suppresses() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            turn_filed_ready_item: true,
            filed_but_didnt_ship_fired: true,
            ..Default::default()
        };
        let build_filed = || "filed".to_string();
        let builders = DirectiveBuilders {
            build_filed_but_didnt_ship_reason: Some(&build_filed),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        assert!(outcome.directive.is_none());
    }

    #[test]
    fn stale_epic_children_branch() {
        let t = active_thread();
        let items = vec![
            item("e-1", WorkItemKind::Epic, WorkItemStatus::Done, None),
            item(
                "c-1",
                WorkItemKind::Task,
                WorkItemStatus::Ready,
                Some("e-1"),
            ),
        ];
        let snap = ThreadSnapshot {
            thread: Some(&t),
            work_items: &items,
            ..Default::default()
        };
        let build_stale = |pairs: &[StaleEpicPair]| {
            format!(
                "stale: {} pairs / {} children",
                pairs.len(),
                pairs[0].stale_children.len()
            )
        };
        let builders = DirectiveBuilders {
            build_stale_epic_children_reason: Some(&build_stale),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, builders);
        let dir = outcome.directive.expect("stale-epic directive");
        assert!(dir.reason.contains("1 pairs / 1 children"));
    }

    #[test]
    fn allow_stop_when_clean() {
        let t = active_thread();
        let snap = ThreadSnapshot {
            thread: Some(&t),
            ..Default::default()
        };
        let outcome = decide_stop_directive(snap, DirectiveBuilders::default());
        assert!(outcome.directive.is_none());
        assert!(outcome.side_effects.is_empty());
    }

    #[test]
    fn find_stale_epic_children_pairs_filters_correctly() {
        let items = vec![
            // closed epic with stale child → flagged
            item("e1", WorkItemKind::Epic, WorkItemStatus::Done, None),
            item("c1", WorkItemKind::Task, WorkItemStatus::Ready, Some("e1")),
            // closed epic with all-done children → not flagged
            item("e2", WorkItemKind::Epic, WorkItemStatus::Done, None),
            item("c2", WorkItemKind::Task, WorkItemStatus::Done, Some("e2")),
            // open epic → never flagged
            item("e3", WorkItemKind::Epic, WorkItemStatus::Ready, None),
            item("c3", WorkItemKind::Task, WorkItemStatus::Ready, Some("e3")),
            // non-epic with closed status → never flagged
            item("t4", WorkItemKind::Task, WorkItemStatus::Done, None),
        ];
        let pairs = find_stale_epic_children_pairs(&items);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].epic.id, WorkItemId::from("e1"));
        assert_eq!(pairs[0].stale_children.len(), 1);
    }

    #[test]
    fn compute_audit_signature_stable_under_reordering() {
        let a = item("wi-a", WorkItemKind::Task, WorkItemStatus::InProgress, None);
        let b = item("wi-b", WorkItemKind::Task, WorkItemStatus::InProgress, None);
        let s1 = compute_audit_signature(&[a.clone(), b.clone()]);
        let s2 = compute_audit_signature(&[b, a]);
        assert_eq!(s1, s2);
    }
}
