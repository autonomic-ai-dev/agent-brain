use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

use anyhow::Result;

pub enum WriteOp {
    StoreMemory {
        resp_tx: std::sync::mpsc::Sender<Result<serde_json::Value>>,
        payload: store_memory_payload::StoreMemoryRequest,
    },
    DeleteMemory {
        resp_tx: std::sync::mpsc::Sender<Result<serde_json::Value>>,
        id: Option<String>,
        topic: Option<String>,
        scope: Option<String>,
        scope_key: Option<String>,
    },
    ReindexComplete,
}

pub mod store_memory_payload {
    #[derive(Debug, Clone)]
    pub struct StoreMemoryRequest {
        pub topic: String,
        pub fact: String,
        pub scope: String,
        pub scope_key: Option<String>,
        pub confidence: f64,
    }
}

pub struct WriteQueue {
    tx: SyncSender<WriteOp>,
}

impl WriteQueue {
    pub fn spawn<F>(handler: F) -> Self
    where
        F: Fn(WriteOp) + Send + 'static,
    {
        let (tx, rx): (SyncSender<WriteOp>, Receiver<WriteOp>) = mpsc::sync_channel(256);
        thread::spawn(move || {
            while let Ok(op) = rx.recv() {
                handler(op);
            }
        });
        Self { tx }
    }

    pub fn send(&self, op: WriteOp) -> Result<()> {
        self.tx
            .send(op)
            .map_err(|e| anyhow::anyhow!("write queue closed: {e}"))
    }
}
