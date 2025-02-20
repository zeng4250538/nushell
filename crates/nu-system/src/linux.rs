use procfs::process::{FDInfo, Io, Process, Stat, Status, TasksIter};
use procfs::{ProcError, ProcessCgroup};
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

pub enum ProcessTask {
    Process(Process),
    Task { stat: Stat, owner: u32 },
}

impl ProcessTask {
    pub fn stat(&self) -> &Stat {
        match self {
            ProcessTask::Process(x) => &x.stat,
            ProcessTask::Task { stat: x, owner: _ } => x,
        }
    }

    pub fn cmdline(&self) -> Result<Vec<String>, ProcError> {
        match self {
            ProcessTask::Process(x) => x.cmdline(),
            _ => Err(ProcError::Other("not supported".to_string())),
        }
    }

    pub fn cgroups(&self) -> Result<Vec<ProcessCgroup>, ProcError> {
        match self {
            ProcessTask::Process(x) => x.cgroups(),
            _ => Err(ProcError::Other("not supported".to_string())),
        }
    }

    pub fn fd(&self) -> Result<Vec<FDInfo>, ProcError> {
        match self {
            ProcessTask::Process(x) => x.fd(),
            _ => Err(ProcError::Other("not supported".to_string())),
        }
    }

    pub fn loginuid(&self) -> Result<u32, ProcError> {
        match self {
            ProcessTask::Process(x) => x.loginuid(),
            _ => Err(ProcError::Other("not supported".to_string())),
        }
    }

    pub fn owner(&self) -> u32 {
        match self {
            ProcessTask::Process(x) => x.owner,
            ProcessTask::Task { stat: _, owner: x } => *x,
        }
    }

    pub fn wchan(&self) -> Result<String, ProcError> {
        match self {
            ProcessTask::Process(x) => x.wchan(),
            _ => Err(ProcError::Other("not supported".to_string())),
        }
    }
}

pub struct ProcessInfo {
    pub pid: i32,
    pub ppid: i32,
    pub curr_proc: ProcessTask,
    pub prev_proc: ProcessTask,
    pub curr_io: Option<Io>,
    pub prev_io: Option<Io>,
    pub curr_status: Option<Status>,
    pub interval: Duration,
}

pub fn collect_proc(interval: Duration, with_thread: bool) -> Vec<ProcessInfo> {
    let mut base_procs = Vec::new();
    let mut base_tasks = HashMap::new();
    let mut ret = Vec::new();

    if let Ok(all_proc) = procfs::process::all_processes() {
        for proc in all_proc {
            let io = proc.io().ok();
            let time = Instant::now();
            if with_thread {
                if let Ok(iter) = proc.tasks() {
                    collect_task(iter, &mut base_tasks);
                }
            }
            base_procs.push((proc.pid(), proc, io, time));
        }
    }

    thread::sleep(interval);

    for (pid, prev_proc, prev_io, prev_time) in base_procs {
        let curr_proc = if let Ok(proc) = Process::new(pid) {
            proc
        } else {
            prev_proc.clone()
        };
        let curr_io = curr_proc.io().ok();
        let curr_status = curr_proc.status().ok();
        let curr_time = Instant::now();
        let interval = curr_time - prev_time;
        let ppid = curr_proc.stat.ppid;
        let owner = curr_proc.owner;

        let mut curr_tasks = HashMap::new();
        if with_thread {
            if let Ok(iter) = curr_proc.tasks() {
                collect_task(iter, &mut curr_tasks);
            }
        }

        let curr_proc = ProcessTask::Process(curr_proc);
        let prev_proc = ProcessTask::Process(prev_proc);

        let proc = ProcessInfo {
            pid,
            ppid,
            curr_proc,
            prev_proc,
            curr_io,
            prev_io,
            curr_status,
            interval,
        };

        ret.push(proc);

        for (tid, (pid, curr_stat, curr_status, curr_io)) in curr_tasks {
            if let Some((_, prev_stat, _, prev_io)) = base_tasks.remove(&tid) {
                let proc = ProcessInfo {
                    pid: tid,
                    ppid: pid,
                    curr_proc: ProcessTask::Task {
                        stat: curr_stat,
                        owner,
                    },
                    prev_proc: ProcessTask::Task {
                        stat: prev_stat,
                        owner,
                    },
                    curr_io,
                    prev_io,
                    curr_status,
                    interval,
                };
                ret.push(proc);
            }
        }
    }

    ret
}

#[allow(clippy::type_complexity)]
fn collect_task(iter: TasksIter, map: &mut HashMap<i32, (i32, Stat, Option<Status>, Option<Io>)>) {
    for task in iter {
        let task = if let Ok(x) = task {
            x
        } else {
            continue;
        };
        if task.tid != task.pid {
            let stat = if let Ok(x) = task.stat() {
                x
            } else {
                continue;
            };
            let status = task.status().ok();
            let io = task.io().ok();
            map.insert(task.tid, (task.pid, stat, status, io));
        }
    }
}

impl ProcessInfo {
    /// PID of process
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Name of command
    pub fn name(&self) -> String {
        self.command()
            .split(' ')
            .collect::<Vec<_>>()
            .first()
            .map(|x| x.to_string())
            .unwrap_or_default()
    }

    /// Full name of command, with arguments
    pub fn command(&self) -> String {
        if let Ok(cmd) = &self.curr_proc.cmdline() {
            if !cmd.is_empty() {
                cmd.join(" ").replace("\n", " ").replace("\t", " ")
            } else {
                self.curr_proc.stat().comm.clone()
            }
        } else {
            self.curr_proc.stat().comm.clone()
        }
    }

    /// Get the status of the process
    pub fn status(&self) -> String {
        match self.curr_proc.stat().state {
            'S' => "Sleeping".into(),
            'R' => "Running".into(),
            'D' => "Disk sleep".into(),
            'Z' => "Zombie".into(),
            'T' => "Stopped".into(),
            't' => "Tracing".into(),
            'X' => "Dead".into(),
            'x' => "Dead".into(),
            'K' => "Wakekill".into(),
            'W' => "Waking".into(),
            'P' => "Parked".into(),
            _ => "Unknown".into(),
        }
    }

    /// CPU usage as a percent of total
    pub fn cpu_usage(&self) -> f64 {
        let curr_time = self.curr_proc.stat().utime + self.curr_proc.stat().stime;
        let prev_time = self.prev_proc.stat().utime + self.prev_proc.stat().stime;
        let usage_ms =
            (curr_time - prev_time) * 1000 / procfs::ticks_per_second().unwrap_or(100) as u64;
        let interval_ms = self.interval.as_secs() * 1000 + u64::from(self.interval.subsec_millis());
        usage_ms as f64 * 100.0 / interval_ms as f64
    }

    /// Memory size in number of bytes
    pub fn mem_size(&self) -> u64 {
        self.curr_proc.stat().rss_bytes().unwrap_or(0) as u64
    }

    /// Virtual memory size in bytes
    pub fn virtual_size(&self) -> u64 {
        self.curr_proc.stat().vsize
    }
}
