//! 可恢复任务协议与确定性参考执行器。
//!
//! 任务实现 [`ResumableTask::step`]，每次只推进一个有界步骤；`Yield` 表示可以安全
//! 在此处暂停，状态留在任务对象里。执行器在步骤之间做优先级抢占：把低优先任务放回
//! ready 队列（对象不销毁），高优先任务先跑，之后从原状态继续。
//!
//! 这是**单线程步进式参考实现**，用于验证协议与抢占语义；不是线程 runtime，也不强杀
//! 正在执行的 `step`。真正的 worker 版本应复用同一 trait。

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use serde::{Deserialize, Serialize};
use trit_core::value::Ternary;

use crate::{
    CancelToken, DeferredTask, Dependency, MAX_TASKS, Resources, RuntimeConfig, RuntimeMetrics,
    ScheduleError, propagate_unknown_deferrals,
};

/// 一次 step 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// 已保存状态，可以在步骤边界暂停。
    Yield,
    /// 任务完成。
    Done,
    /// 任务失败。
    Failed(String),
}

/// step 可读取的上下文。
#[derive(Debug, Clone)]
pub struct StepContext {
    /// 全局取消令牌；任务可自行轮询。
    pub cancel: CancelToken,
}

/// 可恢复任务：`step` 内部状态必须自包含，Yield 后再次调用从原状态继续。
pub trait ResumableTask: Send {
    /// 推进一个有界步骤。
    fn step(&mut self, ctx: &StepContext) -> Progress;
}

/// 一个可恢复任务规格。
pub struct ResumableSpec {
    /// 唯一任务名。
    pub id: String,
    /// 数值越大越优先。
    pub priority: i32,
    /// 三值依赖。
    pub deps: Vec<Dependency>,
    /// 资源需求。
    pub requirements: Resources,
    /// 有状态任务体。
    pub task: Box<dyn ResumableTask>,
}

impl ResumableSpec {
    /// 构造任务（优先级 0、无资源需求）。
    pub fn new(
        id: impl Into<String>,
        deps: Vec<Dependency>,
        task: impl ResumableTask + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            priority: 0,
            deps,
            requirements: Resources::default(),
            task: Box::new(task),
        }
    }

    /// 设置优先级。
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 设置资源需求。
    pub fn with_requirements(mut self, requirements: Resources) -> Self {
        self.requirements = requirements;
        self
    }
}

/// 可恢复执行结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumableReport {
    /// 请求的 worker/并发槽数。
    pub workers: usize,
    /// 完成的任务名（按 id 排序）。
    pub completed: Vec<String>,
    /// 失败任务名与说明。
    pub failed: Option<(String, String)>,
    /// 取消/未执行的任务名（按 id 排序）。
    pub cancelled: Vec<String>,
    /// Unknown 依赖导致延迟的任务。
    pub deferred: Vec<DeferredTask>,
    /// 抢占次数。
    pub preemptions: usize,
    /// 执行的总 step 数。
    pub steps: u64,
    /// 峰值资源占用。
    pub peak_usage: Resources,
    /// 统计快照。
    pub metrics: RuntimeMetrics,
}

impl ResumableReport {
    /// 以 Prometheus 文本导出统计快照。
    pub fn metrics_prometheus(&self) -> String {
        self.metrics.to_prometheus()
    }
}

/// 可恢复执行错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResumableError {
    /// 图验证失败。
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    /// 单任务资源需求超过容量。
    #[error("任务 {task} 需要 {requirements:?}，超过容量 {capacity:?}")]
    Unschedulable {
        /// 任务名。
        task: String,
        /// 需求。
        requirements: Resources,
        /// 容量。
        capacity: Resources,
    },
}

/// 执行可恢复任务图。
///
/// 每一轮最多有 `config.workers` 个任务处于 running；每个 running 任务推进一个 step。
/// 当高优先级任务放不进槽位/资源时，抢占最低优先级的 running 任务（放回 ready，状态保留）。
/// 取消/失败后不再执行新 step；running 任务的状态被丢弃并记为 cancelled。
pub fn run_resumable(
    specs: Vec<ResumableSpec>,
    config: RuntimeConfig,
    cancel: CancelToken,
) -> Result<ResumableReport, ResumableError> {
    let workers = config.workers;
    let capacity = config.capacity;
    if workers == 0 {
        return Err(ScheduleError::NoWorkers.into());
    }
    if specs.len() > MAX_TASKS {
        return Err(ScheduleError::TooManyTasks(specs.len()).into());
    }
    let n = specs.len();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (position, spec) in specs.iter().enumerate() {
        if index.insert(spec.id.clone(), position).is_some() {
            return Err(ScheduleError::DuplicateTask(spec.id.clone()).into());
        }
    }

    let mut true_deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut direct_unknown: Vec<Option<String>> = vec![None; n];
    for (position, spec) in specs.iter().enumerate() {
        for dependency in &spec.deps {
            let Some(&predecessor) = index.get(&dependency.task) else {
                return Err(ScheduleError::UnknownDependency {
                    task: spec.id.clone(),
                    dependency: dependency.task.clone(),
                }
                .into());
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

    let deferred = propagate_unknown_deferrals(&true_deps, direct_unknown, |position| {
        specs[position].id.clone()
    });
    let active_count = deferred.iter().filter(|reason| reason.is_none()).count();
    if let Some(capacity) = capacity {
        for position in 0..n {
            if deferred[position].is_some() {
                continue;
            }
            if !specs[position]
                .requirements
                .fits_in(capacity, Resources::default())
            {
                return Err(ResumableError::Unschedulable {
                    task: specs[position].id.clone(),
                    requirements: specs[position].requirements,
                    capacity,
                });
            }
        }
    }

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
            .map(|position| specs[position].id.as_str())
            .collect();
        remaining.sort_unstable();
        return Err(ScheduleError::Cycle(remaining.join(", ")).into());
    }

    let mut run_indegree = vec![0usize; n];
    for position in 0..n {
        if deferred[position].is_some() {
            continue;
        }
        for &predecessor in &true_deps[position] {
            if deferred[predecessor].is_none() {
                run_indegree[position] += 1;
            }
        }
    }
    let ids: Vec<String> = specs.iter().map(|spec| spec.id.clone()).collect();
    let priorities: Vec<i32> = specs.iter().map(|spec| spec.priority).collect();
    let requirements: Vec<Resources> = specs.iter().map(|spec| spec.requirements).collect();
    let mut tasks: Vec<Option<Box<dyn ResumableTask>>> =
        specs.into_iter().map(|spec| Some(spec.task)).collect();
    let deferred_out: Vec<DeferredTask> = deferred
        .iter()
        .enumerate()
        .filter_map(|(position, reason)| {
            reason.clone().map(|reason| DeferredTask {
                id: ids[position].clone(),
                reason,
            })
        })
        .collect();
    let mut ready: BinaryHeap<(i32, String, usize)> = (0..n)
        .filter(|&position| deferred[position].is_none() && run_indegree[position] == 0)
        .map(|position| (priorities[position], ids[position].clone(), position))
        .collect();
    let mut running: BTreeMap<usize, Box<dyn ResumableTask>> = BTreeMap::new();
    let mut completed = BTreeSet::new();
    let mut cancelled = BTreeSet::new();
    let mut failed: Option<(String, String)> = None;
    let mut used = Resources::default();
    let mut peak = Resources::default();
    let mut preemptions = 0usize;
    let mut steps = 0u64;
    let mut started = 0usize;
    let mut started_tasks = BTreeSet::new();
    let ctx = StepContext {
        cancel: cancel.clone(),
    };

    while !ready.is_empty() || !running.is_empty() {
        if cancel.is_cancelled() || failed.is_some() {
            cancel_pending(&mut ready, &mut running, &mut tasks, &mut cancelled, &ids);
            break;
        }

        // 准入/抢占：高优先任务优先拿槽位或资源。
        while let Some(&(priority, _, position)) = ready.peek() {
            let free = workers.saturating_sub(running.len());
            let fits = free > 0
                && capacity.is_none_or(|capacity| requirements[position].fits_in(capacity, used));
            if fits {
                let (_, _, position) = ready.pop().expect("peek 后仍存在");
                let task = tasks[position].take().expect("ready 任务未被取走");
                used = used
                    .checked_add(requirements[position])
                    .expect("准入检查保证不溢出");
                peak = peak.max(used);
                running.insert(position, task);
                if started_tasks.insert(position) {
                    started += 1;
                }
                continue;
            }
            if preempt(
                &mut running,
                &mut tasks,
                &priorities,
                &ids,
                &requirements,
                &mut used,
                &mut ready,
                priority,
            )
            .is_none()
            {
                break;
            }
            preemptions += 1;
        }

        let mut order: Vec<usize> = running.keys().copied().collect();
        order.sort_by(|left, right| {
            priorities[*right]
                .cmp(&priorities[*left])
                .then_with(|| ids[*left].cmp(&ids[*right]))
        });
        for position in order {
            let Some(task) = running.get_mut(&position) else {
                continue;
            };
            let progress = task.step(&ctx);
            steps += 1;
            match progress {
                Progress::Yield => {}
                Progress::Done => {
                    running.remove(&position);
                    used = used.release(requirements[position]);
                    completed.insert(ids[position].clone());
                    let nexts = dependents[position].clone();
                    for next in nexts {
                        run_indegree[next] -= 1;
                        if run_indegree[next] == 0 {
                            ready.push((priorities[next], ids[next].clone(), next));
                        }
                    }
                }
                Progress::Failed(message) => {
                    running.remove(&position);
                    used = used.release(requirements[position]);
                    failed = Some((ids[position].clone(), message));
                    break;
                }
            }
            if cancel.is_cancelled() || failed.is_some() {
                break;
            }
        }
    }

    cancel_pending(&mut ready, &mut running, &mut tasks, &mut cancelled, &ids);
    let mut cancelled: Vec<String> = cancelled.into_iter().collect();
    cancelled.sort();
    let completed: Vec<String> = completed.into_iter().collect();
    let metrics = RuntimeMetrics {
        started,
        completed: completed.len(),
        failed: usize::from(failed.is_some()),
        cancelled: cancelled.len(),
        deferred: deferred_out.len(),
        preemptions,
        steps,
        peak_usage: peak,
    };
    Ok(ResumableReport {
        workers,
        completed,
        failed,
        cancelled,
        deferred: deferred_out,
        preemptions,
        steps,
        peak_usage: peak,
        metrics,
    })
}

#[allow(clippy::too_many_arguments)]
fn preempt(
    running: &mut BTreeMap<usize, Box<dyn ResumableTask>>,
    tasks: &mut [Option<Box<dyn ResumableTask>>],
    priorities: &[i32],
    ids: &[String],
    requirements: &[Resources],
    used: &mut Resources,
    ready: &mut BinaryHeap<(i32, String, usize)>,
    high_priority: i32,
) -> Option<usize> {
    let victim = running
        .keys()
        .copied()
        .filter(|position| priorities[*position] < high_priority)
        .min_by_key(|position| (priorities[*position], ids[*position].clone()))?;
    let task = running.remove(&victim)?;
    *used = used.release(requirements[victim]);
    tasks[victim] = Some(task);
    ready.push((priorities[victim], ids[victim].clone(), victim));
    Some(victim)
}

fn cancel_pending(
    ready: &mut BinaryHeap<(i32, String, usize)>,
    running: &mut BTreeMap<usize, Box<dyn ResumableTask>>,
    tasks: &mut [Option<Box<dyn ResumableTask>>],
    cancelled: &mut BTreeSet<String>,
    ids: &[String],
) {
    for position in running.keys() {
        cancelled.insert(ids[*position].clone());
    }
    running.clear();
    while let Some((_, id, _)) = ready.pop() {
        cancelled.insert(id);
    }
    for (position, task) in tasks.iter().enumerate() {
        if task.is_some() {
            cancelled.insert(ids[position].clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Counter {
        done: usize,
        target: usize,
        log: Option<Arc<Mutex<Vec<usize>>>>,
    }

    impl Counter {
        fn new(target: usize, log: Option<Arc<Mutex<Vec<usize>>>>) -> Self {
            Self {
                done: 0,
                target,
                log,
            }
        }
    }

    impl ResumableTask for Counter {
        fn step(&mut self, _ctx: &StepContext) -> Progress {
            self.done += 1;
            if let Some(log) = &self.log {
                log.lock().unwrap().push(self.done);
            }
            if self.done >= self.target {
                Progress::Done
            } else {
                Progress::Yield
            }
        }
    }

    struct Once;

    impl ResumableTask for Once {
        fn step(&mut self, _ctx: &StepContext) -> Progress {
            Progress::Done
        }
    }

    struct Fail;

    impl ResumableTask for Fail {
        fn step(&mut self, _ctx: &StepContext) -> Progress {
            Progress::Failed("boom".to_string())
        }
    }

    fn dep(task: &str, required: Ternary) -> Dependency {
        Dependency {
            task: task.to_string(),
            required,
        }
    }

    #[test]
    fn preempted_task_resumes_from_its_own_state() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let low = ResumableSpec::new("low", vec![], Counter::new(10, Some(Arc::clone(&log))))
            .with_priority(0)
            .with_requirements(Resources::new(0, 10));
        let seed = ResumableSpec::new("seed", vec![], Once).with_priority(10);
        let high = ResumableSpec::new(
            "high",
            vec![dep("seed", Ternary::True)],
            Counter::new(1, None),
        )
        .with_priority(20)
        .with_requirements(Resources::new(0, 10));
        let config = RuntimeConfig::new(2).with_capacity(Resources::new(0, 10));
        let report = run_resumable(vec![low, seed, high], config, CancelToken::new()).unwrap();
        assert_eq!(report.completed, vec!["high", "low", "seed"]);
        assert!(report.preemptions >= 1, "{report:?}");
        assert_eq!(*log.lock().unwrap(), (1..=10).collect::<Vec<_>>());
        assert_eq!(report.steps, 12, "seed 1 + high 1 + low 10");
    }

    #[test]
    fn higher_priority_runs_first_with_one_slot() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let make = |id: &str, priority: i32, log: Arc<Mutex<Vec<usize>>>| {
            ResumableSpec::new(id, vec![], Counter::new(1, Some(log))).with_priority(priority)
        };
        let report = run_resumable(
            vec![
                make("low", 0, Arc::clone(&log)),
                make("high", 10, Arc::clone(&log)),
            ],
            RuntimeConfig::new(1),
            CancelToken::new(),
        )
        .unwrap();
        assert_eq!(report.completed, vec!["high", "low"]);
        assert_eq!(report.preemptions, 0);
    }

    #[test]
    fn cancel_stops_future_steps() {
        let token = CancelToken::new();
        token.cancel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let report = run_resumable(
            vec![ResumableSpec::new(
                "a",
                vec![],
                Counter::new(5, Some(Arc::clone(&log))),
            )],
            RuntimeConfig::new(1),
            token,
        )
        .unwrap();
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(report.cancelled, vec!["a"]);
    }

    #[test]
    fn failure_and_unknown_dependencies_are_reported() {
        let failed = run_resumable(
            vec![
                ResumableSpec::new("bad", vec![], Fail),
                ResumableSpec::new("after", vec![dep("bad", Ternary::True)], Once),
            ],
            RuntimeConfig::new(1),
            CancelToken::new(),
        )
        .unwrap();
        assert_eq!(failed.failed, Some(("bad".to_string(), "boom".to_string())));
        assert!(failed.cancelled.contains(&"after".to_string()));

        let deferred = run_resumable(
            vec![
                ResumableSpec::new("a", vec![], Once),
                ResumableSpec::new("b", vec![dep("a", Ternary::Unknown)], Once),
            ],
            RuntimeConfig::new(1),
            CancelToken::new(),
        )
        .unwrap();
        assert_eq!(deferred.deferred.len(), 1);
        assert_eq!(deferred.completed, vec!["a"]);
    }

    #[test]
    fn rejects_cycles_and_over_capacity() {
        let cycle = vec![
            ResumableSpec::new("a", vec![dep("b", Ternary::True)], Once),
            ResumableSpec::new("b", vec![dep("a", Ternary::True)], Once),
        ];
        assert!(matches!(
            run_resumable(cycle, RuntimeConfig::new(1), CancelToken::new()),
            Err(ResumableError::Schedule(ScheduleError::Cycle(_)))
        ));
        let huge =
            vec![ResumableSpec::new("huge", vec![], Once).with_requirements(Resources::new(0, 11))];
        assert!(matches!(
            run_resumable(
                huge,
                RuntimeConfig::new(1).with_capacity(Resources::new(0, 10)),
                CancelToken::new()
            ),
            Err(ResumableError::Unschedulable { .. })
        ));
    }
}
