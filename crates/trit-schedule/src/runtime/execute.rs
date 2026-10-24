use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};

use trit_core::value::Ternary;

use crate::{DeferredTask, MAX_TASKS, ScheduleError, propagate_unknown_deferrals};

use super::types::{
    CancelToken, Job, Resources, RuntimeConfig, RuntimeError, RuntimeMetrics, RuntimeReport,
    RuntimeTask, TaskFailure,
};

struct Shared {
    jobs: Vec<Option<Job>>,
    ready: BinaryHeap<(i32, String, usize)>,
    dependents: Vec<Vec<usize>>,
    indegree: Vec<usize>,
    running: usize,
    started: usize,
    stop: bool,
    completed: BTreeSet<String>,
    cancelled: BTreeSet<String>,
    failed: Option<TaskFailure>,
    ids: Vec<String>,
    priorities: Vec<i32>,
    requirements: Vec<Resources>,
    used: Resources,
    capacity: Option<Resources>,
    peak: Resources,
    deferred: Vec<DeferredTask>,
}

impl Shared {
    fn cancel_pending(&mut self) {
        while let Some((_, id, _)) = self.ready.pop() {
            self.cancelled.insert(id);
        }
    }
}

/// 按依赖执行任务，使用 `workers` 个 worker。
///
/// `Unknown` 依赖对应的任务不会执行，并沿必需依赖传播；`False` 可选依赖忽略。
/// 失败或取消后不再启动新任务；已经运行的任务需自行响应取消令牌。
pub fn run(
    tasks: Vec<RuntimeTask>,
    workers: usize,
    cancel: CancelToken,
) -> Result<RuntimeReport, RuntimeError> {
    run_with_config(tasks, RuntimeConfig::new(workers), cancel)
}

/// 带资源配置的 runtime。
///
/// 配额是**严格优先级准入**：队首任务放不下时等待，不为了塞进低优先任务而跳过它；
/// 运行中的任务不会被抢占。
pub fn run_with_config(
    tasks: Vec<RuntimeTask>,
    config: RuntimeConfig,
    cancel: CancelToken,
) -> Result<RuntimeReport, RuntimeError> {
    let workers = config.workers;
    let capacity = config.capacity;
    if workers == 0 {
        return Err(ScheduleError::NoWorkers.into());
    }
    if tasks.len() > MAX_TASKS {
        return Err(ScheduleError::TooManyTasks(tasks.len()).into());
    }
    let n = tasks.len();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (position, task) in tasks.iter().enumerate() {
        if index.insert(task.id.clone(), position).is_some() {
            return Err(ScheduleError::DuplicateTask(task.id.clone()).into());
        }
    }

    let mut true_deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut direct_unknown: Vec<Option<String>> = vec![None; n];
    for (position, task) in tasks.iter().enumerate() {
        for dependency in &task.deps {
            let Some(&predecessor) = index.get(&dependency.task) else {
                return Err(ScheduleError::UnknownDependency {
                    task: task.id.clone(),
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
        tasks[position].id.clone()
    });
    let active_count = deferred.iter().filter(|reason| reason.is_none()).count();
    if let Some(capacity) = capacity {
        for position in 0..n {
            if deferred[position].is_some() {
                continue;
            }
            if !tasks[position]
                .requirements
                .fits_in(capacity, Resources::default())
            {
                return Err(RuntimeError::Unschedulable {
                    task: tasks[position].id.clone(),
                    requirements: tasks[position].requirements,
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
            .map(|position| tasks[position].id.as_str())
            .collect();
        remaining.sort_unstable();
        return Err(ScheduleError::Cycle(remaining.join(", ")).into());
    }

    // 上面的 `indegree` 已被 Kahn 归零；runtime 需要一份独立的剩余依赖计数。
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
    let ready: BinaryHeap<(i32, String, usize)> = (0..n)
        .filter(|&position| deferred[position].is_none() && run_indegree[position] == 0)
        .map(|position| {
            (
                tasks[position].priority,
                tasks[position].id.clone(),
                position,
            )
        })
        .collect();
    let ids: Vec<String> = tasks.iter().map(|task| task.id.clone()).collect();
    let priorities: Vec<i32> = tasks.iter().map(|task| task.priority).collect();
    let requirements: Vec<Resources> = tasks.iter().map(|task| task.requirements).collect();
    let jobs: Vec<Option<Job>> = tasks.into_iter().map(|task| Some(task.run)).collect();
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

    if active_count == 0 {
        return Ok(RuntimeReport {
            workers,
            completed: Vec::new(),
            failed: None,
            cancelled: Vec::new(),
            deferred: deferred_out,
            peak_usage: Resources::default(),
            metrics: RuntimeMetrics {
                started: 0,
                completed: 0,
                failed: 0,
                cancelled: 0,
                deferred: 0,
                preemptions: 0,
                steps: 0,
                peak_usage: Resources::default(),
            },
        });
    }

    let shared = Arc::new((
        Mutex::new(Shared {
            jobs,
            ready,
            dependents,
            indegree: run_indegree,
            running: 0,
            started: 0,
            stop: false,
            completed: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            failed: None,
            ids,
            priorities,
            requirements,
            used: Resources::default(),
            capacity,
            peak: Resources::default(),
            deferred: deferred_out,
        }),
        Condvar::new(),
    ));
    let effective_workers = workers.min(active_count);
    std::thread::scope(|scope| {
        for _ in 0..effective_workers {
            let shared = Arc::clone(&shared);
            let cancel = cancel.clone();
            scope.spawn(move || worker(shared, cancel));
        }
    });

    let mut state = shared
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let leftover: Vec<String> = state
        .jobs
        .iter()
        .enumerate()
        .filter_map(|(position, job)| job.as_ref().map(|_| state.ids[position].clone()))
        .collect();
    for id in leftover {
        state.cancelled.insert(id);
    }
    let metrics = RuntimeMetrics {
        started: state.started,
        completed: state.completed.len(),
        failed: usize::from(state.failed.is_some()),
        cancelled: state.cancelled.len(),
        deferred: state.deferred.len(),
        preemptions: 0,
        steps: state.started as u64,
        peak_usage: state.peak,
    };
    Ok(RuntimeReport {
        workers,
        completed: state.completed.iter().cloned().collect(),
        failed: state.failed.clone(),
        cancelled: state.cancelled.iter().cloned().collect(),
        deferred: state.deferred.clone(),
        peak_usage: state.peak,
        metrics,
    })
}

fn worker(shared: Arc<(Mutex<Shared>, Condvar)>, cancel: CancelToken) {
    let (lock, ready_cv) = &*shared;
    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if cancel.is_cancelled() && !state.stop {
            state.stop = true;
            state.cancel_pending();
        }
        let fits = state.ready.peek().is_some_and(|(_, _, position)| {
            state
                .capacity
                .is_none_or(|capacity| state.requirements[*position].fits_in(capacity, state.used))
        });
        if fits {
            let Some((_priority, id, position)) = state.ready.pop() else {
                continue;
            };
            if state.stop {
                state.cancelled.insert(id);
                continue;
            }
            let Some(run) = state.jobs[position].take() else {
                continue;
            };
            // 复审 H05：`capacity = None` 时不做单任务预检，两个 u32::MAX 需求并发
            // 累计仍会溢出——不能用 `expect`（worker panic 毒化锁后其余 worker
            // 无法可靠收尾）。溢出按任务失败处理：停止本轮并通知所有 worker。
            let requirements = state.requirements[position];
            let Some(new_used) = state.used.checked_add(requirements) else {
                state.failed = Some(TaskFailure {
                    task: id,
                    message: "资源累计溢出".to_string(),
                });
                state.stop = true;
                state.cancel_pending();
                ready_cv.notify_all();
                continue;
            };
            state.running += 1;
            state.started += 1;
            state.used = new_used;
            state.peak = state.peak.max(state.used);
            drop(state);

            let result = catch_unwind(AssertUnwindSafe(|| run(&cancel)))
                .unwrap_or_else(|_| Err("任务 panic".to_string()));

            state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.running -= 1;
            state.used = state.used.release(state.requirements[position]);
            match result {
                Ok(()) => {
                    state.completed.insert(id);
                    if !state.stop {
                        let nexts = state.dependents[position].clone();
                        for next in nexts {
                            state.indegree[next] -= 1;
                            if state.indegree[next] == 0 {
                                let task_id = state.ids[next].clone();
                                let priority = state.priorities[next];
                                state.ready.push((priority, task_id, next));
                            }
                        }
                    }
                }
                Err(message) => {
                    state.failed = Some(TaskFailure { task: id, message });
                    state.stop = true;
                    state.cancel_pending();
                }
            }
            ready_cv.notify_all();
            continue;
        }
        if state.stop || state.running == 0 {
            return;
        }
        state = ready_cv
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}
