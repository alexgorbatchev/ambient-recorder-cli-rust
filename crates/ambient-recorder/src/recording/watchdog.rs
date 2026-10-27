use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Independent of the capture thread and diagnostic writer. A blocked stderr
/// cannot delay the supervisor-restart exit beyond the diagnostic allowance.
pub struct Watchdog {
    pulse: SyncSender<()>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Watchdog {
    pub fn new(
        timeout: Duration,
        diagnostic: impl FnOnce() + Send + 'static,
        exit: impl FnOnce(i32) + Send + 'static,
    ) -> Self {
        let (pulse, receiver) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let worker = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                match receiver.recv_timeout(timeout) {
                    Ok(()) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if flag.load(Ordering::Acquire) {
                            return;
                        }
                        let (done, wait) = mpsc::sync_channel(1);
                        thread::spawn(move || {
                            diagnostic();
                            let _ = done.send(());
                        });
                        let _ = wait.recv_timeout(Duration::from_millis(100));
                        exit(2);
                        return;
                    }
                }
            }
        });
        Self {
            pulse,
            stopped,
            worker: Some(worker),
        }
    }
    pub fn native(timeout: Duration) -> Self {
        Self::new(
            timeout,
            move || {
                let _ = writeln!(
                    std::io::stderr(),
                    "ERR: Recorder blocked for {timeout:?}; exiting for supervisor restart"
                );
            },
            |code| std::process::exit(code),
        )
    }
    pub fn pulse(&self) {
        let _ = self.pulse.try_send(());
    }
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.pulse();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}
