use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;
use tracing::debug;

use crate::{client::ThingsCloudClient, wire::wire_object::WireObject};

pub trait CloudWriter {
    fn commit(&mut self, changes: BTreeMap<String, WireObject>) -> Result<i64>;

    fn head_index(&self) -> i64;
}

pub struct LoggingCloudWriter {
    inner: Box<dyn CloudWriter>,
}

impl LoggingCloudWriter {
    pub fn new(inner: Box<dyn CloudWriter>) -> Self {
        Self { inner }
    }
}

impl CloudWriter for LoggingCloudWriter {
    fn commit(&mut self, changes: BTreeMap<String, WireObject>) -> Result<i64> {
        // a commit builds on the client's head
        // that is the synced head or the one the previous commit returned
        let ancestor_index = self.inner.head_index();
        let uuids = changes.keys().cloned().collect::<Vec<_>>();
        // the payload is serialized for the log only when that log is on
        let request_json = if tracing::enabled!(target: "things_cli::cloud_commit::request", tracing::Level::DEBUG)
        {
            serde_json::to_string(&json!({
                "ancestor_index": ancestor_index,
                "changes": &changes,
            }))
            .unwrap_or_else(|error| json!({ "error": error.to_string() }).to_string())
        } else {
            String::new()
        };
        debug!(
            target: "things_cli::cloud_commit::request",
            event = "cloud.commit.request",
            ancestor_index,
            change_count = uuids.len(),
            uuids = ?uuids,
            request_json = %request_json,
            "cloud commit request"
        );

        match self.inner.commit(changes) {
            Ok(head_index) => {
                debug!(
                    target: "things_cli::cloud_commit::success",
                    event = "cloud.commit.success",
                    ancestor_index,
                    change_count = uuids.len(),
                    uuids = ?uuids,
                    head_index,
                    "cloud commit succeeded"
                );
                Ok(head_index)
            }
            // the command returns the error
            // the event is for a debug log
            Err(err) => {
                debug!(
                    target: "things_cli::cloud_commit::error",
                    event = "cloud.commit.error",
                    ancestor_index,
                    change_count = uuids.len(),
                    uuids = ?uuids,
                    error = %err,
                    "cloud commit failed"
                );
                Err(err)
            }
        }
    }

    fn head_index(&self) -> i64 {
        self.inner.head_index()
    }
}

/// commits against the history and head of the client that synchronized this run's state
pub struct LiveCloudWriter {
    client: ThingsCloudClient,
}

#[derive(Default)]
pub struct DryRunCloudWriter {
    head_index: i64,
}

impl DryRunCloudWriter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LiveCloudWriter {
    pub fn new(client: ThingsCloudClient) -> Self {
        Self { client }
    }
}

impl CloudWriter for LiveCloudWriter {
    fn commit(&mut self, changes: BTreeMap<String, WireObject>) -> Result<i64> {
        self.client.commit(changes)
    }

    fn head_index(&self) -> i64 {
        self.client.head_index
    }
}

impl CloudWriter for DryRunCloudWriter {
    fn commit(&mut self, _changes: BTreeMap<String, WireObject>) -> Result<i64> {
        self.head_index += 1;
        Ok(self.head_index)
    }

    fn head_index(&self) -> i64 {
        self.head_index
    }
}
