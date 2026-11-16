//! Work off the window's thread: each job runs on one of Rune's shared workers and its answer
//! comes back through a queue the app drains in `poll`, after the waker asks for a frame.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use rune::Waker;

pub struct Jobs<E> {
    tx: Sender<E>,
    rx: Receiver<E>,
    /// The window's waker, once it has one. Shared, so a job started before then (the first
    /// listing of a folder named on the command line) still asks for a frame when it is done.
    waker: Arc<Mutex<Option<Waker>>>,
    /// Run each job where it is asked for, so a headless run or a test has its answer at once.
    inline: bool,
}

impl<E: Send + 'static> Jobs<E> {
    pub fn new(inline: bool) -> Jobs<E> {
        let (tx, rx) = channel();
        Jobs { tx, rx, waker: Arc::default(), inline }
    }

    pub fn is_inline(&self) -> bool {
        self.inline
    }

    pub fn set_waker(&self, waker: Waker) {
        if let Ok(mut slot) = self.waker.lock() {
            *slot = Some(waker);
        }
    }

    /// Asks the window for a frame, from any thread: it polls the app before drawing.
    pub fn waker(&self) -> Waker {
        let slot = self.waker.clone();
        Arc::new(move || {
            // Called outside the lock: the waker runs the window's own code.
            let waker = slot.lock().ok().and_then(|slot| slot.clone());
            if let Some(wake) = waker {
                wake();
            }
        })
    }

    /// A way to queue an answer from any thread; call [`Jobs::waker`]'s waker after doing so.
    pub fn sender(&self) -> Sender<E> {
        self.tx.clone()
    }

    pub fn spawn(&self, name: &str, work: impl FnOnce() -> E + Send + 'static) {
        if self.inline {
            let _ = self.tx.send(work());
            return;
        }
        let (tx, wake) = (self.sender(), self.waker());
        let run = move || {
            let _ = tx.send(work());
            wake();
        };
        if let Err(e) = rune::task::spawn(name, run) {
            eprintln!("code: could not start {name}: {e}");
        }
    }

    pub fn try_recv(&self) -> Option<E> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn a_job_started_before_the_window_has_a_waker_still_wakes_it() {
        let jobs: Jobs<u32> = Jobs::new(false);
        let (release, held) = channel::<()>();
        jobs.spawn("early", move || {
            let _ = held.recv();
            7
        });
        let wakes = Arc::new(AtomicUsize::new(0));
        let counted = wakes.clone();
        jobs.set_waker(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        release.send(()).expect("the job is waiting");
        let deadline = Instant::now() + Duration::from_secs(10);
        while wakes.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the job never woke the window");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(jobs.try_recv(), Some(7), "its answer is queued before the wake");
    }

    #[test]
    fn an_inline_job_has_its_answer_at_once() {
        let jobs: Jobs<u32> = Jobs::new(true);
        jobs.spawn("now", || 3);
        assert_eq!(jobs.try_recv(), Some(3));
        assert_eq!(jobs.try_recv(), None);
    }
}
