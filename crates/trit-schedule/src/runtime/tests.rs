use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::{Dependency, ScheduleError};

use super::*;

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn dep(task: &str, required: Ternary) -> Dependency {
        Dependency {
            task: task.to_string(),
            required,
        }
    }

    #[test]
    fn chain_runs_in_dependency_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let task = |id: &str, deps: Vec<Dependency>, log: Arc<Mutex<Vec<String>>>| {
            let id = id.to_string();
            let logged = id.clone();
            RuntimeTask::new(id, deps, move |_| {
                log.lock().unwrap().push(logged.clone());
                Ok(())
            })
        };
        let tasks = vec![
            task("a", vec![], Arc::clone(&log)),
            task("b", vec![dep("a", Ternary::True)], Arc::clone(&log)),
            task("c", vec![dep("b", Ternary::True)], Arc::clone(&log)),
        ];
        let report = run(tasks, 1, CancelToken::new()).unwrap();
        assert_eq!(*log.lock().unwrap(), vec!["a", "b", "c"]);
        assert_eq!(report.completed, vec!["a", "b", "c"]);
        assert!(report.failed.is_none());
        assert!(report.cancelled.is_empty());
    }

    #[test]
    fn two_workers_run_independent_tasks_concurrently() {
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let make = |id: &str| {
            let active = Arc::clone(&active);
            let max = Arc::clone(&max);
            RuntimeTask::new(id, vec![], move |_| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(40));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
        };
        let report = run(vec![make("a"), make("b")], 2, CancelToken::new()).unwrap();
        assert_eq!(report.completed.len(), 2);
        assert!(max.load(Ordering::SeqCst) >= 2, "两个 worker 应真正重叠");
    }

    #[test]
    fn cancellation_prevents_unstarted_tasks() {
        let token = CancelToken::new();
        token.cancel();
        let ran = Arc::new(AtomicBool::new(false));
        let make = |id: &str| {
            let ran = Arc::clone(&ran);
            RuntimeTask::new(id, vec![], move |_| {
                ran.store(true, Ordering::SeqCst);
                Ok(())
            })
        };
        let report = run(vec![make("a"), make("b")], 2, token).unwrap();
        assert!(!ran.load(Ordering::SeqCst));
        assert!(report.completed.is_empty());
        assert_eq!(report.cancelled, vec!["a", "b"]);
    }

    #[test]
    fn failure_stops_scheduling_and_cancels_dependents() {
        let tasks = vec![
            RuntimeTask::new("ok", vec![], |_| Ok(())),
            RuntimeTask::new("bad", vec![], |_| Err("boom".to_string())),
            RuntimeTask::new("after", vec![dep("bad", Ternary::True)], |_| Ok(())),
        ];
        let report = run(tasks, 2, CancelToken::new()).unwrap();
        assert_eq!(report.failed.unwrap().task, "bad");
        assert!(report.cancelled.contains(&"after".to_string()));
    }

    #[test]
    fn unknown_dependency_is_deferred_not_run() {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = Arc::clone(&ran);
        let tasks = vec![
            RuntimeTask::new("a", vec![], |_| Ok(())),
            RuntimeTask::new("b", vec![dep("a", Ternary::Unknown)], move |_| {
                ran_clone.store(true, Ordering::SeqCst);
                Ok(())
            }),
        ];
        let report = run(tasks, 2, CancelToken::new()).unwrap();
        assert_eq!(report.completed, vec!["a"]);
        assert_eq!(report.deferred.len(), 1);
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn higher_priority_starts_first_with_one_worker() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let make = |id: &str, priority: i32, log: Arc<Mutex<Vec<String>>>| {
            let id = id.to_string();
            let logged = id.clone();
            RuntimeTask::new(id, vec![], move |_| {
                log.lock().unwrap().push(logged.clone());
                Ok(())
            })
            .with_priority(priority)
        };
        let tasks = vec![
            make("low", 0, Arc::clone(&log)),
            make("high", 10, Arc::clone(&log)),
        ];
        run(tasks, 1, CancelToken::new()).unwrap();
        assert_eq!(*log.lock().unwrap(), vec!["high", "low"]);
    }

    #[test]
    fn rejects_bad_graphs_and_panics_are_task_failures() {
        let cycle = vec![
            RuntimeTask::new("a", vec![dep("b", Ternary::True)], |_| Ok(())),
            RuntimeTask::new("b", vec![dep("a", Ternary::True)], |_| Ok(())),
        ];
        assert!(matches!(
            run(cycle, 1, CancelToken::new()),
            Err(RuntimeError::Schedule(ScheduleError::Cycle(_)))
        ));
        assert!(matches!(
            run(Vec::new(), 0, CancelToken::new()),
            Err(RuntimeError::Schedule(ScheduleError::NoWorkers))
        ));
        let panicking = vec![RuntimeTask::new("boom", vec![], |_| panic!("test panic"))];
        let report = run(panicking, 1, CancelToken::new()).unwrap();
        assert_eq!(report.failed.unwrap().task, "boom");
    }

    #[test]
    fn join_waits_for_all_predecessors() {
        let completed = Arc::new(AtomicUsize::new(0));
        let a = RuntimeTask::new("a", vec![], {
            let completed = Arc::clone(&completed);
            move |_| {
                std::thread::sleep(Duration::from_millis(10));
                completed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let b = RuntimeTask::new("b", vec![], {
            let completed = Arc::clone(&completed);
            move |_| {
                std::thread::sleep(Duration::from_millis(60));
                completed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let c = RuntimeTask::new(
            "c",
            vec![dep("a", Ternary::True), dep("b", Ternary::True)],
            {
                let completed = Arc::clone(&completed);
                move |_| {
                    if completed.load(Ordering::SeqCst) == 2 {
                        Ok(())
                    } else {
                        Err("join 在前置完成前启动".to_string())
                    }
                }
            },
        );
        let report = run(vec![a, b, c], 2, CancelToken::new()).unwrap();
        assert!(report.failed.is_none(), "{report:?}");
        assert_eq!(report.completed, vec!["a", "b", "c"]);
    }

    #[test]
    fn failure_does_not_start_new_work() {
        let started = Arc::new(AtomicBool::new(false));
        let a = RuntimeTask::new("a", vec![], {
            let started = Arc::clone(&started);
            move |_| {
                started.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                Ok(())
            }
        });
        let bad = RuntimeTask::new("bad", vec![], {
            let started = Arc::clone(&started);
            move |_| {
                while !started.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                Err("boom".to_string())
            }
        });
        let after = RuntimeTask::new("after", vec![dep("a", Ternary::True)], |_| Ok(()));
        let report = run(vec![a, bad, after], 2, CancelToken::new()).unwrap();
        assert_eq!(report.failed.unwrap().task, "bad");
        assert!(!report.completed.contains(&"after".to_string()));
        assert!(report.cancelled.contains(&"after".to_string()));
    }

    #[test]
    fn quota_serializes_tasks_that_do_not_fit_together() {
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let make = |id: &str| {
            let active = Arc::clone(&active);
            let max = Arc::clone(&max);
            RuntimeTask::new(id, vec![], move |_| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .with_requirements(Resources::new(0, 6))
        };
        let config = RuntimeConfig::new(2).with_capacity(Resources::new(0, 10));
        let report =
            run_with_config(vec![make("a"), make("b")], config, CancelToken::new()).unwrap();
        assert_eq!(report.completed, vec!["a", "b"]);
        assert_eq!(max.load(Ordering::SeqCst), 1, "6+6 超过 10，必须串行");
        assert_eq!(report.peak_usage.memory, 6);
    }

    #[test]
    fn task_exceeding_capacity_is_rejected() {
        let task =
            RuntimeTask::new("huge", vec![], |_| Ok(())).with_requirements(Resources::new(0, 11));
        let config = RuntimeConfig::new(1).with_capacity(Resources::new(0, 10));
        assert!(matches!(
            run_with_config(vec![task], config, CancelToken::new()),
            Err(RuntimeError::Unschedulable { .. })
        ));
    }

    #[test]
    fn quota_accumulates_across_tasks() {
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let make = |id: &str| {
            let active = Arc::clone(&active);
            let max = Arc::clone(&max);
            RuntimeTask::new(id, vec![], move |_| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .with_requirements(Resources::new(0, 4))
        };
        let config = RuntimeConfig::new(3).with_capacity(Resources::new(0, 8));
        let report = run_with_config(
            vec![make("a"), make("b"), make("c")],
            config,
            CancelToken::new(),
        )
        .unwrap();
        assert_eq!(report.completed.len(), 3);
        assert_eq!(
            max.load(Ordering::SeqCst),
            2,
            "4+4+4 在容量 8 下只能并行两个"
        );
        assert_eq!(report.peak_usage.memory, 8);
    }

    #[test]
    fn priority_admission_wins_under_quota() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let make = |id: &str, priority: i32, log: Arc<Mutex<Vec<String>>>| {
            let id = id.to_string();
            let logged = id.clone();
            RuntimeTask::new(id, vec![], move |_| {
                log.lock().unwrap().push(logged.clone());
                Ok(())
            })
            .with_priority(priority)
            .with_requirements(Resources::new(0, 6))
        };
        let tasks = vec![
            make("low", 0, Arc::clone(&log)),
            make("high", 10, Arc::clone(&log)),
        ];
        let config = RuntimeConfig::new(2).with_capacity(Resources::new(0, 6));
        run_with_config(tasks, config, CancelToken::new()).unwrap();
        assert_eq!(*log.lock().unwrap(), vec!["high", "low"]);
    }
}

#[cfg(test)]
mod metrics_export_tests {
    use super::*;

    #[test]
    fn prometheus_exports_all_fields() {
        let metrics = RuntimeMetrics {
            started: 3,
            completed: 2,
            failed: 1,
            cancelled: 4,
            deferred: 5,
            preemptions: 6,
            steps: 7,
            peak_usage: Resources::new(8, 9),
        };
        let text = metrics.to_prometheus_with("trit_test");
        assert!(text.contains("# TYPE trit_test_started gauge"));
        assert!(text.contains("trit_test_started 3\n"));
        assert!(text.contains("trit_test_completed 2\n"));
        assert!(text.contains("trit_test_failed 1\n"));
        assert!(text.contains("trit_test_cancelled 4\n"));
        assert!(text.contains("trit_test_deferred 5\n"));
        assert!(text.contains("trit_test_preemptions 6\n"));
        assert!(text.contains("trit_test_steps 7\n"));
        assert!(text.contains("trit_test_peak_cpu 8\n"));
        assert!(text.contains("trit_test_peak_memory 9\n"));
        assert!(!text.contains("trit_schedule_"));
        assert!(metrics.to_prometheus().contains("trit_schedule_steps 7\n"));
        assert!(
            metrics
                .to_prometheus_with("")
                .contains("trit_schedule_started 3\n")
        );
    }

    #[test]
    fn resource_overflow_without_capacity_fails_instead_of_panicking() {
        // 复审 H05：capacity=None 不做单任务预检，两个 u32::MAX 需求并发累计
        // 曾在 worker 里 `expect` panic——毒化锁后其余 worker 无法收尾、调用方挂死。
        // 堆序保证 b 先被弹出；让 b 睡 100ms 持有 used=MAX，a 被第二个 worker
        // 弹出时必然溢出（否则两者跑完 used 归零，测不到溢出分支）。
        let b = RuntimeTask::new("b", vec![], |_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            Ok(())
        })
        .with_requirements(Resources::new(u32::MAX, 0));
        let a = RuntimeTask::new("a", vec![], |_| Ok(()))
            .with_requirements(Resources::new(u32::MAX, 0));
        let report = run(vec![a, b], 2, CancelToken::new()).unwrap();
        let failure = report
            .failed
            .expect("资源累计溢出必须判失败而不是 panic/挂起");
        assert_eq!(failure.task, "a", "b 先占满，a 溢出");
        assert_eq!(failure.message, "资源累计溢出");
        assert_eq!(report.completed, vec!["b"]);
    }
}
