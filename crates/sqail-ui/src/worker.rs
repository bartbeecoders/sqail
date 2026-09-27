//! Background async work. The UI thread never blocks: it spawns futures on a
//! tokio runtime and receives [`Msg`]s over a channel, waking egui to repaint.

use std::future::Future;
use std::sync::mpsc;

use crate::app::Msg;

pub struct Worker {
    rt: tokio::runtime::Runtime,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    ctx: egui::Context,
}

/// Cloneable handle for sending messages from background tasks.
#[derive(Clone)]
pub struct Sink {
    tx: mpsc::Sender<Msg>,
    ctx: egui::Context,
}

impl Sink {
    pub fn send(&self, msg: Msg) {
        // The receiver only goes away when the app is closing.
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("sqail-worker")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, rx) = mpsc::channel();
        Self { rt, tx, rx, ctx }
    }

    pub fn sink(&self) -> Sink {
        Sink {
            tx: self.tx.clone(),
            ctx: self.ctx.clone(),
        }
    }

    /// Run `fut`; its output is delivered as a message.
    pub fn run(&self, fut: impl Future<Output = Msg> + Send + 'static) {
        let sink = self.sink();
        self.rt.spawn(async move { sink.send(fut.await) });
    }

    /// Run a task that may send any number of messages.
    pub fn spawn<F>(&self, f: impl FnOnce(Sink) -> F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.rt.spawn(f(self.sink()));
    }

    /// Fire-and-forget work whose result the UI does not need.
    pub fn detach(&self, fut: impl Future<Output = ()> + Send + 'static) {
        self.rt.spawn(fut);
    }

    /// Messages that arrived since the last frame.
    pub fn drain(&self) -> impl Iterator<Item = Msg> + '_ {
        self.rx.try_iter()
    }
}
