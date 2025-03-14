use futures::{
    channel::oneshot::{Receiver, channel},
    lock::Mutex,
};
use send_wrapper::SendWrapper;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::{File, MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::{ConvertMeta, FileWrap, Options, TaskRequest, WorkerMessage, worker::ConvertError};

use super::task::BlobUrl;

#[derive(Debug, Clone)]
pub(crate) struct Converter {
    worker: SendWrapper<Worker>,
    ready: Arc<Mutex<Option<Receiver<()>>>>,
}

impl Converter {
    pub(crate) fn new() -> Self {
        log::debug!("spawning worker");
        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        let worker: Worker =
            Worker::new_with_options("./worker_loader.js", &opts).expect("failed to spawn worker");

        let (ready_tx, ready_rx) = channel::<()>();
        let worker_ = worker.clone();
        let on_message = Closure::once(move |ev: MessageEvent| {
            match serde_wasm_bindgen::from_value(ev.data()) {
                Ok(WorkerMessage::WorkerReady) => ready_tx.send(()).unwrap(),
                Ok(msg) => log::warn!("unexpected message {:?}", msg),
                Err(err) => log::error!("failed to parse message {:?}", err),
            }
            worker_.set_onmessage(None);
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        Self {
            worker: SendWrapper::new(worker),
            ready: Mutex::new(Some(ready_rx)).into(),
        }
    }

    pub(crate) async fn convert(
        &self,
        options: Options,
        files: Vec<File>,
    ) -> Result<(BlobUrl, ConvertMeta), ConvertError> {
        // wait for worker ready
        let mut ready = self.ready.lock().await;
        // we deliberately hold the lock unit task done
        if let Some(ready) = ready.take() {
            log::debug!("convert: wait for worker ready");
            ready.await?;
        }
        log::debug!("convert: {:?} files", files.len());
        // setup event listener
        let (result_tx, result_rx) = channel();
        let worker = self.worker.clone().take();
        let on_message = Closure::once(move |ev: MessageEvent| {
            match serde_wasm_bindgen::from_value(ev.data()) {
                Ok(WorkerMessage::TaskDone(result)) => result_tx.send(result).unwrap(),
                Ok(msg) => log::warn!("unexpected message {:?}", msg),
                Err(err) => log::error!("failed to parse message {:?}", err),
            }
            worker.set_onmessage(None);
        });
        let worker = self.worker.clone().take();
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        // send request
        let request = TaskRequest {
            options,
            files: files.into_iter().map(FileWrap).collect(),
        };
        worker.post_message(&serde_wasm_bindgen::to_value(&request).unwrap())?;
        // wait response
        result_rx.await?.map(|r| (BlobUrl::new(r.file_url), r.meta))
    }
}
