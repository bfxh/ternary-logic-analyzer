//! 三值依赖的静态 DAG 调度器。
//!
//! 依赖边有三种语义：
//! - `True`：前置任务必须完成；
//! - `False`：可选依赖，忽略；
//! - `Unknown`：前置状态未知，任务（以及依赖它的任务）被**延迟**，不猜成可运行。
//!
//! 调度是静态的：给定任务图与 worker 数，输出每个任务的开始/完成时间、worker、
//! makespan 与关键路径。它不负责实际执行、抢占或故障恢复；那些是后续 runtime 的事。

#![forbid(unsafe_code)]

pub mod checkpoint;
pub mod resumable;
pub mod resumable_threaded;
pub mod runtime;

pub use checkpoint::{
    CHECKPOINT_VERSION, Checkpoint, CheckpointEntry, CheckpointError, CheckpointIndex,
    CheckpointStore, CheckpointTask, CheckpointedTask, FileCheckpointStore, TaskRestorer,
    restore_checkpoint, restore_specs, save_checkpoint,
};
pub use resumable::{
    Progress, ResumableError, ResumableReport, ResumableSpec, ResumableTask, StepContext,
    run_resumable,
};
pub use resumable_threaded::run_resumable_threaded;
pub use runtime::{
    CancelToken, Resources, RuntimeConfig, RuntimeError, RuntimeMetrics, RuntimeReport,
    RuntimeTask, TaskFailure, run as run_runtime, run_with_config,
};

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use serde::{Deserialize, Serialize};
use trit_core::value::Ternary;

/// 单次输入的任务数上限，防止无界 JSON 直接吃光内存。
pub const MAX_TASKS: usize = 1_000_000;

fn default_true() -> Ternary {
    Ternary::True
}

fn default_workers() -> usize {
    1
}

/// 一个调度任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// 唯一任务名。
    pub id: String,
    /// 执行时长（任意统一时间单位）。
    pub duration: u32,
    /// 数值越大越优先；默认 0。
    #[serde(default)]
    pub priority: i32,
    /// 前置依赖。
    #[serde(default)]
    pub deps: Vec<Dependency>,
}

/// 一条三值依赖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    /// 前置任务名。
    pub task: String,
    /// `True` 必需、`False` 可选、`Unknown` 未知；默认 `True`。
    #[serde(default = "default_true")]
    pub required: Ternary,
}

/// 调度输入（CLI JSON 也直接使用这个结构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleInput {
    /// worker 数；默认 1。
    #[serde(default = "default_workers")]
    pub workers: usize,
    /// 任务列表。
    #[serde(default)]
    pub tasks: Vec<Task>,
}

/// 一个已排定任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// 任务名。
    pub id: String,
    /// 开始时间。
    pub start: u64,
    /// 完成时间。
    pub finish: u64,
    /// worker 下标（0 起）。
    pub worker: usize,
    /// 优先级。
    pub priority: i32,
}

/// 任务为什么被延迟。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum DeferReason {
    /// 直接依赖了状态未知的任务。
    UnknownDependency(String),
    /// 必需的某个前置任务已被延迟。
    BlockedBy(String),
}

/// 直接未知依赖 + 沿 True 边传播到不动点——工作表传播，O(V+E)。
///
/// 旧实现每轮全量扫描所有任务，反拓扑长链上每轮只推进一层，O(V²)；
/// 公开上限 100 万项时可被构造成计算型 DoS（复审 H04）。传播闭包与旧实现一致；
/// `BlockedBy` 的原因取标记时按声明序找到的第一个已延期前驱（诊断信息）。
pub(crate) fn propagate_unknown_deferrals(
    true_deps: &[Vec<usize>],
    direct_unknown: Vec<Option<String>>,
    mut id_of: impl FnMut(usize) -> String,
) -> Vec<Option<DeferReason>> {
    let n = true_deps.len();
    // 反向邻接：dependents[u] = 经 True 边依赖 u 的任务。
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (position, predecessors) in true_deps.iter().enumerate() {
        for &predecessor in predecessors {
            dependents[predecessor].push(position);
        }
    }
    let mut deferred: Vec<Option<DeferReason>> = direct_unknown
        .into_iter()
        .map(|name| name.map(DeferReason::UnknownDependency))
        .collect();
    let mut worklist: Vec<usize> = (0..n).filter(|&index| deferred[index].is_some()).collect();
    while let Some(source) = worklist.pop() {
        for &position in &dependents[source] {
            if deferred[position].is_some() {
                continue;
            }
            deferred[position] = true_deps[position]
                .iter()
                .find(|&&predecessor| deferred[predecessor].is_some())
                .map(|&predecessor| DeferReason::BlockedBy(id_of(predecessor)));
            worklist.push(position);
        }
    }
    deferred
}

/// 一个被延迟的任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredTask {
    /// 任务名。
    pub id: String,
    /// 延迟原因。
    pub reason: DeferReason,
}

/// 调度结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// 请求的 worker 数。
    pub workers: usize,
    /// 所有已排定任务的最大完成时间；没有可排任务时为 0。
    pub makespan: u64,
    /// 忽略 worker 数的关键路径长度（makespan 的下界）。
    pub critical_path: u64,
    /// 已排定任务，按开始时间/任务名排序。
    pub scheduled: Vec<ScheduledTask>,
    /// 被延迟的任务，按任务名排序。
    pub deferred: Vec<DeferredTask>,
}

/// 调度错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    /// worker 数必须至少为 1。
    #[error("workers 必须 >= 1")]
    NoWorkers,
    /// 任务数超过上限。
    #[error("任务数 {0} 超过上限 {MAX_TASKS}")]
    TooManyTasks(usize),
    /// 任务名重复。
    #[error("任务名重复：{0}")]
    DuplicateTask(String),
    /// 依赖引用了不存在的任务。
    #[error("任务 {task} 依赖不存在的任务 {dependency}")]
    UnknownDependency {
        /// 引用方。
        task: String,
        /// 不存在的前置任务。
        dependency: String,
    },
    /// 必需依赖形成环。
    #[error("必需依赖存在环：{0}")]
    Cycle(String),
    /// 完成时间溢出。
    #[error("任务 {0} 的完成时间溢出")]
    Overflow(String),
}

/// 从 [`ScheduleInput`] 调度。
pub fn schedule_input(input: &ScheduleInput) -> Result<Schedule, ScheduleError> {
    schedule(&input.tasks, input.workers)
}

/// 静态调度 `tasks`，使用 `workers` 个并行 worker。
///
/// 只有 `required == True` 的边参与排序和关键路径；`False` 完全忽略；
/// `Unknown` 会让任务延迟，并把“必需依赖它”的任务一起延迟。
pub fn schedule(tasks: &[Task], workers: usize) -> Result<Schedule, ScheduleError> {
    if workers == 0 {
        return Err(ScheduleError::NoWorkers);
    }
    if tasks.len() > MAX_TASKS {
        return Err(ScheduleError::TooManyTasks(tasks.len()));
    }
    let n = tasks.len();
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    for (position, task) in tasks.iter().enumerate() {
        if index.insert(task.id.as_str(), position).is_some() {
            return Err(ScheduleError::DuplicateTask(task.id.clone()));
        }
    }

    let mut true_deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut direct_unknown: Vec<Option<String>> = vec![None; n];
    for (position, task) in tasks.iter().enumerate() {
        for dependency in &task.deps {
            let Some(&predecessor) = index.get(dependency.task.as_str()) else {
                return Err(ScheduleError::UnknownDependency {
                    task: task.id.clone(),
                    dependency: dependency.task.clone(),
                });
            };
            match dependency.required {
                Ternary::True => true_deps[position].push(predecessor),
                Ternary::Unknown => {
                    if direct_unknown[position].is_none() {
                        direct_unknown[position] = Some(dependency.task.clone());
                    }
                }
                Ternary::False => {}
            }
        }
    }

    // 直接未知 + 沿 True 边传播。
    let deferred = propagate_unknown_deferrals(&true_deps, direct_unknown, |position| {
        tasks[position].id.clone()
    });

    let active_count = deferred.iter().filter(|reason| reason.is_none()).count();
    let mut indegree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for position in 0..n {
        if deferred[position].is_some() {
            continue;
        }
        for &predecessor in &true_deps[position] {
            if deferred[predecessor].is_none() {
                indegree[position] += 1;
                dependents[predecessor].push(position);
            }
        }
    }

    let mut queue: VecDeque<usize> = (0..n)
        .filter(|&position| deferred[position].is_none() && indegree[position] == 0)
        .collect();
    let mut topological = Vec::with_capacity(active_count);
    while let Some(position) = queue.pop_front() {
        topological.push(position);
        for &next in &dependents[position] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    if topological.len() != active_count {
        let mut remaining: Vec<&str> = (0..n)
            .filter(|&position| deferred[position].is_none() && !topological.contains(&position))
            .map(|position| tasks[position].id.as_str())
            .collect();
        remaining.sort_unstable();
        return Err(ScheduleError::Cycle(remaining.join(", ")));
    }

    let mut longest = vec![0u64; n];
    for &position in &topological {
        let mut start = 0u64;
        for &predecessor in &true_deps[position] {
            if deferred[predecessor].is_none() {
                start = start.max(longest[predecessor]);
            }
        }
        longest[position] = start + u64::from(tasks[position].duration);
    }
    let critical_path = topological
        .iter()
        .map(|&position| longest[position])
        .max()
        .unwrap_or(0);

    let effective_workers = workers.min(active_count.max(1));
    let mut free: BTreeSet<usize> = (0..effective_workers).collect();
    let mut schedule_indegree = vec![0usize; n];
    for position in 0..n {
        if deferred[position].is_some() {
            continue;
        }
        for &predecessor in &true_deps[position] {
            if deferred[predecessor].is_none() {
                schedule_indegree[position] += 1;
            }
        }
    }
    let mut ready: BTreeSet<(i32, u32, String, usize)> = (0..n)
        .filter(|&position| deferred[position].is_none() && schedule_indegree[position] == 0)
        .map(|position| ready_key(&tasks[position], position))
        .collect();
    let mut running: BinaryHeap<Reverse<(u64, usize, usize)>> = BinaryHeap::new();
    let mut scheduled = Vec::with_capacity(active_count);
    let mut now = 0u64;

    while scheduled.len() < active_count {
        while let Some(&Reverse((finish, worker, position))) = running.peek() {
            if finish > now {
                break;
            }
            running.pop();
            free.insert(worker);
            for &next in &dependents[position] {
                schedule_indegree[next] -= 1;
                if schedule_indegree[next] == 0 {
                    ready.insert(ready_key(&tasks[next], next));
                }
            }
        }

        while !ready.is_empty() {
            let Some(worker) = free.pop_first() else {
                break;
            };
            let Some((priority, duration, id, position)) = ready.pop_last() else {
                break;
            };
            let finish = now
                .checked_add(u64::from(duration))
                .ok_or_else(|| ScheduleError::Overflow(id.clone()))?;
            scheduled.push(ScheduledTask {
                id,
                start: now,
                finish,
                worker,
                priority,
            });
            running.push(Reverse((finish, worker, position)));
        }

        if scheduled.len() >= active_count {
            break;
        }
        let Some(&Reverse((finish, _, _))) = running.peek() else {
            break;
        };
        now = finish;
    }

    if scheduled.len() != active_count {
        return Err(ScheduleError::Cycle(
            "内部错误：仍有可运行任务未排定".to_string(),
        ));
    }
    let makespan = scheduled.iter().map(|task| task.finish).max().unwrap_or(0);
    scheduled.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut deferred_out: Vec<DeferredTask> = deferred
        .iter()
        .enumerate()
        .filter_map(|(position, reason)| {
            reason.clone().map(|reason| DeferredTask {
                id: tasks[position].id.clone(),
                reason,
            })
        })
        .collect();
    deferred_out.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Schedule {
        workers,
        makespan,
        critical_path,
        scheduled,
        deferred: deferred_out,
    })
}

fn ready_key(task: &Task, position: usize) -> (i32, u32, String, usize) {
    (task.priority, task.duration, task.id.clone(), position)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(task: &str, required: Ternary) -> Dependency {
        Dependency {
            task: task.to_string(),
            required,
        }
    }

    fn task(id: &str, duration: u32, deps: Vec<Dependency>) -> Task {
        Task {
            id: id.to_string(),
            duration,
            priority: 0,
            deps,
        }
    }

    #[test]
    fn chain_uses_one_worker_and_full_critical_path() {
        let tasks = vec![
            task("a", 3, vec![]),
            task("b", 2, vec![dep("a", Ternary::True)]),
            task("c", 4, vec![dep("b", Ternary::True)]),
        ];
        let report = schedule(&tasks, 1).unwrap();
        assert_eq!(report.makespan, 9);
        assert_eq!(report.critical_path, 9);
        assert_eq!(report.scheduled.len(), 3);
        assert_eq!(report.scheduled[0].start, 0);
        assert_eq!(report.scheduled[1].start, 3);
        assert_eq!(report.scheduled[2].start, 5);
        assert!(report.deferred.is_empty());
    }

    #[test]
    fn two_workers_overlap_independent_tasks() {
        let tasks = vec![
            task("a", 3, vec![]),
            task("b", 2, vec![]),
            task(
                "c",
                1,
                vec![dep("a", Ternary::True), dep("b", Ternary::True)],
            ),
        ];
        let serial = schedule(&tasks, 1).unwrap();
        let parallel = schedule(&tasks, 2).unwrap();
        assert_eq!(serial.makespan, 6);
        assert_eq!(parallel.makespan, 4);
        assert_eq!(parallel.critical_path, 4);
        assert_eq!(parallel.scheduled[2].start, 3);
    }

    #[test]
    fn unknown_dependency_defers_transitively() {
        let tasks = vec![
            task("a", 1, vec![]),
            task("b", 1, vec![dep("a", Ternary::Unknown)]),
            task("c", 1, vec![dep("b", Ternary::True)]),
            task("d", 1, vec![dep("a", Ternary::False)]),
        ];
        let report = schedule(&tasks, 2).unwrap();
        assert_eq!(report.scheduled.len(), 2);
        assert_eq!(report.deferred.len(), 2);
        assert_eq!(report.deferred[0].id, "b");
        assert_eq!(
            report.deferred[0].reason,
            DeferReason::UnknownDependency("a".to_string())
        );
        assert_eq!(report.deferred[1].id, "c");
        assert_eq!(
            report.deferred[1].reason,
            DeferReason::BlockedBy("b".to_string())
        );
    }

    #[test]
    fn optional_dependency_does_not_wait() {
        let tasks = vec![
            task("slow", 10, vec![]),
            task("quick", 1, vec![dep("slow", Ternary::False)]),
        ];
        let report = schedule(&tasks, 2).unwrap();
        assert_eq!(report.makespan, 10);
        let quick = report
            .scheduled
            .iter()
            .find(|task| task.id == "quick")
            .unwrap();
        assert_eq!(quick.start, 0);
    }

    #[test]
    fn higher_priority_is_scheduled_first() {
        let tasks = vec![
            Task {
                id: "low".to_string(),
                duration: 1,
                priority: 0,
                deps: vec![],
            },
            Task {
                id: "high".to_string(),
                duration: 1,
                priority: 10,
                deps: vec![],
            },
        ];
        let report = schedule(&tasks, 1).unwrap();
        assert_eq!(report.scheduled[0].id, "high");
    }

    #[test]
    fn rejects_cycles_duplicates_unknown_refs_and_zero_workers() {
        let cycle = vec![
            task("a", 1, vec![dep("b", Ternary::True)]),
            task("b", 1, vec![dep("a", Ternary::True)]),
        ];
        assert!(matches!(schedule(&cycle, 1), Err(ScheduleError::Cycle(_))));
        let duplicate = vec![task("a", 1, vec![]), task("a", 2, vec![])];
        assert!(matches!(
            schedule(&duplicate, 1),
            Err(ScheduleError::DuplicateTask(_))
        ));
        let unknown = vec![task("a", 1, vec![dep("missing", Ternary::True)])];
        assert!(matches!(
            schedule(&unknown, 1),
            Err(ScheduleError::UnknownDependency { .. })
        ));
        assert_eq!(schedule(&[], 0).unwrap_err(), ScheduleError::NoWorkers);
    }

    #[test]
    fn parses_json_with_ternary_dependencies() {
        let input: ScheduleInput = serde_json::from_str(
            r#"{
                "workers": 2,
                "tasks": [
                    {"id":"a","duration":3},
                    {"id":"b","duration":2,"deps":[{"task":"a","required":"True"}]},
                    {"id":"c","duration":1,"deps":[{"task":"a","required":"Unknown"}]}
                ]
            }"#,
        )
        .unwrap();
        let report = schedule_input(&input).unwrap();
        assert_eq!(report.makespan, 5);
        assert_eq!(report.deferred.len(), 1);
        assert_eq!(
            report.deferred[0].reason,
            DeferReason::UnknownDependency("a".to_string())
        );
    }

    #[test]
    fn reverse_chain_deferral_is_linear_not_quadratic() {
        // 复审 H04：反拓扑链上旧不动点扫描每轮只推进一层，O(V²)——
        // 10 万项反向链就是 10^10 次内层访问。工作表传播 O(V+E) 必须秒过。
        const N: usize = 100_000;
        let mut tasks: Vec<Task> = (0..N)
            .map(|index| {
                let deps = if index + 1 < N {
                    vec![dep(&format!("t{}", index + 1), Ternary::True)]
                } else {
                    vec![dep("src", Ternary::Unknown)]
                };
                task(&format!("t{index}"), 1, deps)
            })
            .collect();
        tasks.push(task("src", 1, vec![]));

        let report = schedule(&tasks, 2).unwrap();
        // 只有 src（无依赖）能排定；整条链都被延期。
        assert_eq!(
            report
                .scheduled
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["src"]
        );
        assert_eq!(report.deferred.len(), N);
        let by_id = |name: &str| {
            report
                .deferred
                .iter()
                .find(|entry| entry.id == name)
                .unwrap_or_else(|| panic!("{name} 未出现在 deferred"))
        };
        assert_eq!(
            by_id("t99999").reason,
            DeferReason::UnknownDependency("src".to_string())
        );
        assert_eq!(by_id("t0").reason, DeferReason::BlockedBy("t1".to_string()));
        assert_eq!(by_id("t1").reason, DeferReason::BlockedBy("t2".to_string()));
    }
}
