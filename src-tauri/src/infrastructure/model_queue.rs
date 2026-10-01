//! Who goes first at the language model.
//!
//! llama-server answers one request at a time, in the order they arrive. A
//! talk's requests are the ones a learner sits waiting on. Everything else the
//! model does — scoring a finished talk, reading a placement's level, getting
//! the next talk's instructions ready — can wait, and on a classroom laptop
//! it takes long enough to matter: scoring one talk is the best part of a
//! minute, and a chore opened while the talk before it was being scored had its
//! warm-up queued behind the scoring. Its first reply took 58 s.
//!
//! So a talk always goes first. The rest are errands: they run while no talk
//! is open, one at a time and the most urgent first, and they are made of small
//! pieces. Between pieces an errand asks whether something should go ahead of
//! it, and gives way if so. A talk opened in the middle of an errand waits for
//! the piece in hand, a few seconds at most, not for the whole errand, and the
//! errand carries on once the talk is over.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How long a talk stays open after its last request. A talk is over when the
/// learner finishes it. One left open, or abandoned for Home, holds errands
/// back for this long after its last request and then stops. Errands must not
/// run inside a talk: one would replace the talk's prompt in llama.cpp's slot,
/// and the talk's next reply would evaluate all of it again.
pub const TALK_QUIET: Duration = Duration::from_secs(180);

/// How long a waiting errand sleeps before it looks again unprompted. Every
/// change that could let it go wakes it sooner.
const RECHECK: Duration = Duration::from_secs(30);

/// Work for the model that nobody is waiting on as it happens, most urgent
/// first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Errand {
    /// Reading a finished talk: what it showed and its one fix, or the level a
    /// placement found. The recap shows them.
    Assess,
    /// Getting a talk the learner is likely to open next ready, so its first
    /// reply does not wait for its instructions to be evaluated.
    Prepare,
}

const KINDS: usize = 2;

/// Why an errand should stop and let something else have the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GiveWay {
    /// A talk wants the model, or one is open.
    Talk,
    /// A more urgent errand is waiting.
    Errand,
    /// Ella is closing.
    Closing,
}

/// How a run of an errand ended.
pub enum Step<T> {
    Done(T),
    /// It stopped to let something else go first, and is to be run again when
    /// its turn comes back.
    GaveWay,
}

pub struct ModelQueue {
    state: Mutex<State>,
    changed: Condvar,
    quiet: Duration,
}

#[derive(Default)]
struct State {
    /// A talk's requests in flight: warm-ups, replies, a placement chat's
    /// checks.
    talking: usize,
    /// When the talk the learner is in last asked for anything; `None` once
    /// it is over.
    talk: Option<Instant>,
    /// Errands waiting their turn, by kind.
    waiting: [usize; KINDS],
    /// The errand that has the model now.
    running: Option<Errand>,
    closed: bool,
}

impl State {
    fn talk_open(&self, quiet: Duration) -> Option<Duration> {
        let since = self.talk?.elapsed();
        (since < quiet).then(|| quiet - since)
    }

    fn outranked(&self, kind: Errand) -> bool {
        self.waiting[..kind as usize].iter().any(|&waiting| waiting > 0)
    }
}

impl Default for ModelQueue {
    fn default() -> Self {
        Self::new(TALK_QUIET)
    }
}

impl ModelQueue {
    /// `quiet` is how long a talk stays open after its last request:
    /// `TALK_QUIET`, shorter in tests.
    pub fn new(quiet: Duration) -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            quiet,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A talk's request is going to the model. Hold the guard until it has
    /// its answer: the errand running now gives way at its next piece, and no
    /// other starts until the talk is over.
    pub fn talking(self: &Arc<Self>) -> Talking {
        let mut state = self.lock();
        state.talking += 1;
        state.talk = Some(Instant::now());
        Talking(Arc::clone(self))
    }

    /// The talk the learner was in is over, so errands held back for it can go.
    pub fn talk_over(&self) {
        self.lock().talk = None;
        self.changed.notify_all();
    }

    /// Ella is closing. Errands waiting their turn give up, and one running
    /// stops at its next piece.
    pub fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    /// Runs `errand` once nothing should go ahead of it, and again each time
    /// it gives way, until it is done. `None` if Ella closed first.
    ///
    /// The errand is handed an `ErrandTurn` to ask between its pieces whether
    /// to give way. When to stop is up to the errand. An errand stopped by a
    /// talk may also decide it is no longer wanted and return `Done`, as
    /// preparing for a talk does: the talk changes what comes next.
    pub fn run<T>(&self, kind: Errand, mut errand: impl FnMut(&ErrandTurn<'_>) -> Step<T>) -> Option<T> {
        let mut state = self.lock();
        state.waiting[kind as usize] += 1;
        loop {
            loop {
                if state.closed {
                    state.waiting[kind as usize] -= 1;
                    drop(state);
                    self.changed.notify_all();
                    return None;
                }
                let wait = if state.running.is_some() || state.talking > 0 || state.outranked(kind) {
                    Some(RECHECK)
                } else {
                    state.talk_open(self.quiet)
                };
                match wait {
                    Some(wait) => {
                        state = self
                            .changed
                            .wait_timeout(state, wait)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                    None => break,
                }
            }
            state.waiting[kind as usize] -= 1;
            state.running = Some(kind);
            drop(state);

            let step = errand(&ErrandTurn { queue: self, kind });

            state = self.lock();
            state.running = None;
            self.changed.notify_all();
            match step {
                Step::Done(value) => return Some(value),
                Step::GaveWay => state.waiting[kind as usize] += 1,
            }
        }
    }
}

/// Held while a talk's request is with the model.
pub struct Talking(Arc<ModelQueue>);

impl Drop for Talking {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.talking = state.talking.saturating_sub(1);
        // Still open: the talk's next request comes once the learner answers.
        // A talk declared over while this was in flight stays over.
        if state.talk.is_some() {
            state.talk = Some(Instant::now());
        }
        drop(state);
        self.0.changed.notify_all();
    }
}

/// An errand's turn at the model.
pub struct ErrandTurn<'a> {
    queue: &'a ModelQueue,
    kind: Errand,
}

impl ErrandTurn<'_> {
    /// Whether to stop here and let something else have the model. Asked
    /// between pieces.
    pub fn give_way(&self) -> Option<GiveWay> {
        let state = self.queue.lock();
        if state.closed {
            Some(GiveWay::Closing)
        } else if state.talking > 0 || state.talk_open(self.queue.quiet).is_some() {
            Some(GiveWay::Talk)
        } else if state.outranked(self.kind) {
            Some(GiveWay::Errand)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    const SOON: Duration = Duration::from_millis(150);

    fn queue(quiet: Duration) -> Arc<ModelQueue> {
        Arc::new(ModelQueue::new(quiet))
    }

    /// An errand of `pieces` pieces, each `piece` long, that gives way when
    /// told to. It reports each piece it finishes.
    fn errand(
        queue: &Arc<ModelQueue>,
        kind: Errand,
        pieces: usize,
        piece: Duration,
        done: mpsc::Sender<(Errand, usize)>,
    ) -> thread::JoinHandle<Option<usize>> {
        let queue = Arc::clone(queue);
        thread::spawn(move || {
            let mut next = 0;
            let mut runs = 0;
            queue.run(kind, |turn| {
                runs += 1;
                while next < pieces {
                    if turn.give_way().is_some() {
                        return Step::GaveWay;
                    }
                    thread::sleep(piece);
                    next += 1;
                    done.send((kind, next)).unwrap();
                }
                Step::Done(runs)
            })
        })
    }

    #[test]
    fn an_errand_waits_while_a_talk_is_open_and_goes_once_it_is_over() {
        let queue = queue(Duration::from_secs(60));
        let talking = queue.talking();
        let (done, pieces) = mpsc::channel();
        let errand = errand(&queue, Errand::Assess, 1, Duration::ZERO, done);
        assert!(pieces.recv_timeout(SOON).is_err(), "nothing runs beside a talk's request");
        drop(talking);
        assert!(pieces.recv_timeout(SOON).is_err(), "nor between the talk's requests");
        queue.talk_over();
        assert_eq!(pieces.recv_timeout(Duration::from_secs(2)).unwrap(), (Errand::Assess, 1));
        assert_eq!(errand.join().unwrap(), Some(1));
    }

    #[test]
    fn a_talk_nobody_finishes_stops_holding_errands_once_it_has_been_quiet() {
        let queue = queue(Duration::from_millis(300));
        drop(queue.talking());
        let started = Instant::now();
        let (done, pieces) = mpsc::channel();
        let errand = errand(&queue, Errand::Prepare, 1, Duration::ZERO, done);
        pieces.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250), "it waited out the quiet");
        errand.join().unwrap();
    }

    #[test]
    fn an_errand_gives_way_to_a_talk_at_its_next_piece_and_carries_on_after_it() {
        let queue = queue(Duration::from_secs(60));
        let (done, pieces) = mpsc::channel();
        let errand = errand(&queue, Errand::Assess, 4, Duration::from_millis(100), done);
        assert_eq!(pieces.recv_timeout(Duration::from_secs(2)).unwrap().1, 1);

        let talking = queue.talking();
        // Pieces finished before the talk came in, then the piece in hand, if
        // there was one; after that, nothing.
        let mut last = pieces.try_iter().map(|(_, piece)| piece).last().unwrap_or(1);
        if let Ok((_, piece)) = pieces.recv_timeout(Duration::from_millis(300)) {
            last = piece;
        }
        assert!(pieces.recv_timeout(Duration::from_millis(300)).is_err(), "it gave way");
        assert!(last < 4, "it stopped before the end");
        drop(talking);
        queue.talk_over();

        while last < 4 {
            last = pieces.recv_timeout(Duration::from_secs(2)).unwrap().1;
        }
        assert_eq!(errand.join().unwrap(), Some(2), "run twice: before the talk and after it");
    }

    #[test]
    fn reading_a_finished_talk_goes_before_getting_the_next_one_ready() {
        let queue = queue(Duration::from_secs(60));
        let (done, pieces) = mpsc::channel();
        let prepare = errand(&queue, Errand::Prepare, 3, Duration::from_millis(100), done.clone());
        assert_eq!(pieces.recv_timeout(Duration::from_secs(2)).unwrap(), (Errand::Prepare, 1));

        let assess = errand(&queue, Errand::Assess, 2, Duration::from_millis(50), done);
        // Preparing's other two pieces and the assessment's two, in whatever
        // order they came.
        let order: Vec<(Errand, usize)> = (0..4)
            .map(|_| pieces.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        let assessed = order.iter().rposition(|(kind, _)| *kind == Errand::Assess).unwrap();
        let resumed = order.iter().position(|(kind, piece)| *kind == Errand::Prepare && *piece == 3).unwrap();
        assert!(assessed < resumed, "the assessment finished first: {order:?}");
        assert_eq!(assess.join().unwrap(), Some(1));
        assert_eq!(prepare.join().unwrap(), Some(2));
    }

    #[test]
    fn closing_lets_every_errand_go() {
        let queue = queue(Duration::from_secs(60));
        let _talking = queue.talking();
        let (done, _pieces) = mpsc::channel();
        let waiting = errand(&queue, Errand::Assess, 1, Duration::ZERO, done);
        thread::sleep(SOON);
        queue.close();
        assert_eq!(waiting.join().unwrap(), None);

        let ran = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&ran);
        assert_eq!(
            queue.run(Errand::Prepare, move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Step::Done(())
            }),
            None,
            "nothing starts once Ella is closing"
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_talk_declared_over_mid_request_stays_over() {
        let queue = queue(Duration::from_secs(60));
        let talking = queue.talking();
        queue.talk_over();
        drop(talking);
        let (done, pieces) = mpsc::channel();
        let errand = errand(&queue, Errand::Assess, 1, Duration::ZERO, done);
        pieces.recv_timeout(Duration::from_secs(2)).unwrap();
        errand.join().unwrap();
    }
}
