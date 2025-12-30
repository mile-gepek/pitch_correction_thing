//! Thread signaling and waiting using [`Signal`].
//!
//! # Examples
//! ```
//! let (_, signal) = Signal::spawn(|signal| {
//!     let _handle = signal.wait(); // Repeatedly parks until [`set`] is called from a different thread.
//!     // Do stuff...
//!     // When _handle is dropped, the signal resets and unparks.
//! });
//!
//! // Ready to start other thread
//! signal.set();
//! ```

use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
    thread::{self, Builder, JoinHandle, Thread},
};

/// Thread signal for waiting for conditions between threads.
///
/// The methods [`spawn`] and [`spawn_with_builder`] create a thread with a function
/// that accepts a [`Signal`], which can be used to [`wait`] until a different thread
/// calls [`set`].
///
/// Any thread can call the methods, but only the spawned thread will wait.
///
/// The signal is cleared once the [`SignalHandle`] returned by [`wait`] is dropped.
///
/// # Examples
/// ```
/// let (_, signal) = Signal::spawn(|signal| {
///     let _handle = signal.wait(); // Repeatedly parks until [`set`] is called from a different thread.
///     // Do stuff...
///     // When _handle is dropped, the signal resets and unparks.
/// });
///
/// // Ready to start other thread
/// signal.set();
/// ```
///
/// [`spawn`]: Self::spawn
/// [`spawn_with_builder`]: Self::spawn_with_builder
/// [`wait`]: Self::wait
/// [`set`]: Self::set
pub struct Signal {
    thread: thread::Thread,
    flag: Arc<AtomicBool>,
}

impl Signal {
    fn new(thread: Thread, flag: Arc<AtomicBool>) -> Self {
        Self { thread, flag }
    }

    pub fn spawn<F, T>(f: F) -> (JoinHandle<T>, Signal)
    where
        F: FnOnce(Signal) -> T,
        F: Send + 'static,
        T: Send + 'static,
    {
        let builder = Builder::new();
        Self::spawn_with_builder(builder, f).unwrap()
    }

    pub fn spawn_with_builder<F, T>(builder: Builder, f: F) -> io::Result<(JoinHandle<T>, Signal)>
    where
        F: FnOnce(Signal) -> T,
        F: Send + 'static,
        T: Send + 'static,
    {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();
        let thread = builder.spawn(move || {
            let thread = thread::current();
            let signal = Signal::new(thread, flag_clone);
            f(signal)
        })?;
        let signal = Signal::new(thread.thread().clone(), flag);
        Ok((thread, signal))
    }

    /// Signal the waiting thread to continue.
    pub fn set(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
        self.thread.unpark();
    }

    /// Make the spawned thread wait until [`set`] is called.
    pub fn wait(&self) -> SignalHandle<'_> {
        let handle = SignalHandle::new(self);
        handle.wait();
        handle
    }

    /// Resets the internal flag, so the signal doesn't exit instantly on the next wait.
    ///
    /// Called automatically when a [`SignalHandle`] is dropped.
    fn reset(&self) {
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[must_use = "The signal will automatically reset if not used"]
pub struct SignalHandle<'a> {
    signal: &'a Signal,
}

impl<'a> SignalHandle<'a> {
    fn new(signal: &'a Signal) -> Self {
        Self { signal }
    }
}

impl SignalHandle<'_> {
    pub fn wait(&self) {
        while !self.signal.flag.load(std::sync::atomic::Ordering::Acquire) {
            thread::park();
        }
    }
}

impl Drop for SignalHandle<'_> {
    fn drop(&mut self) {
        self.signal.reset();
    }
}
