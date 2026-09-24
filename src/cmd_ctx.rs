use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    rc::Rc,
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};

use crate::{
    app::Cli,
    client::ThingsCloudClient,
    cloud_writer::{CloudWriter, DryRunCloudWriter, LiveCloudWriter, LoggingCloudWriter},
    ids::ThingsId,
    wire::wire_object::WireObject,
};

pub trait CmdCtx {
    fn now_timestamp(&self) -> f64;
    fn today_timestamp(&self) -> i64;
    fn today(&self) -> DateTime<Utc> {
        let ts = self.today_timestamp();
        Utc.timestamp_opt(ts, 0)
            .single()
            .unwrap_or_else(Utc::now)
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|d| Utc.from_utc_datetime(&d))
            .unwrap_or_else(Utc::now)
    }
    fn next_id(&mut self) -> String;
    fn commit_changes(
        &mut self,
        changes: BTreeMap<String, WireObject>,
        ancestor_index: Option<i64>,
    ) -> Result<i64>;
    fn current_head_index(&self) -> i64;
}

#[derive(Default)]
pub struct DefaultCmdCtx {
    no_cloud: bool,
    today_ts_override: Option<i64>,
    now_ts_override: Option<f64>,
    id_seed: Option<u64>,
    ids_issued: u64,
    /// the client that synchronized this run's state
    ///
    /// taken by the first write
    /// that write commits against the history and head the state came from
    cloud: Rc<RefCell<Option<ThingsCloudClient>>>,
    /// the objects whose replay did not complete
    ///
    /// never written through
    degraded: Rc<RefCell<HashSet<ThingsId>>>,
    writer: Option<Box<dyn CloudWriter>>,
}

impl DefaultCmdCtx {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            no_cloud: cli.no_cloud,
            today_ts_override: cli.today_ts,
            now_ts_override: cli.now_ts,
            id_seed: cli.id_seed,
            ids_issued: 0,
            cloud: Rc::clone(&cli.cloud),
            degraded: Rc::clone(&cli.degraded),
            writer: None,
        }
    }

    fn writer_mut(&mut self) -> Result<&mut dyn CloudWriter> {
        if self.writer.is_none() {
            let inner: Box<dyn CloudWriter> = if self.no_cloud {
                Box::new(DryRunCloudWriter::new())
            } else {
                let client = self.cloud.borrow_mut().take().ok_or_else(|| {
                    anyhow!("Not writing to Things Cloud: this run did not synchronize with it.")
                })?;
                Box::new(LiveCloudWriter::new(client))
            };
            self.writer = Some(Box::new(LoggingCloudWriter::new(inner)));
        }
        Ok(self.writer.as_deref_mut().expect("writer initialized"))
    }
}

impl CmdCtx for DefaultCmdCtx {
    fn now_timestamp(&self) -> f64 {
        self.now_ts_override
            .unwrap_or_else(crate::common::now_ts_f64)
    }

    fn today_timestamp(&self) -> i64 {
        self.today_ts_override
            .unwrap_or_else(|| crate::common::today_utc().timestamp())
    }

    fn next_id(&mut self) -> String {
        let Some(seed) = self.id_seed else {
            return ThingsId::random().to_string();
        };
        self.ids_issued += 1;
        ThingsId::from_u128((u128::from(seed) << 64) | u128::from(self.ids_issued)).to_string()
    }

    fn commit_changes(
        &mut self,
        changes: BTreeMap<String, WireObject>,
        ancestor_index: Option<i64>,
    ) -> Result<i64> {
        // a write on top of a state that is behind the history would overwrite what the CLI never saw
        for uuid in changes.keys() {
            if uuid
                .parse::<ThingsId>()
                .is_ok_and(|id| self.degraded.borrow().contains(&id))
            {
                return Err(anyhow!(
                    "Not writing {uuid}: its history did not replay completely, THINGS_LOG=warn has the reason."
                ));
            }
        }
        self.writer_mut()?.commit(changes, ancestor_index)
    }

    fn current_head_index(&self) -> i64 {
        self.writer.as_deref().map_or(0, CloudWriter::head_index)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::wire::{task::TaskPatch, wire_object::EntityType};

    #[test]
    fn a_write_to_an_object_that_did_not_replay_is_refused() {
        let cli = Cli::parse_from(["things", "--no-cloud"]);
        let id = ThingsId::from_u128(7);
        cli.degraded.borrow_mut().insert(id.clone());
        let mut ctx = DefaultCmdCtx::from_cli(&cli);
        let patch = || {
            WireObject::update(
                EntityType::Task7,
                TaskPatch {
                    title: Some("x".to_string()),
                    ..Default::default()
                },
            )
        };

        let refused = ctx
            .commit_changes(BTreeMap::from([(id.to_string(), patch())]), None)
            .expect_err("refused");
        assert!(
            refused
                .to_string()
                .starts_with(&format!("Not writing {id}"))
        );

        let other = ThingsId::from_u128(8).to_string();
        ctx.commit_changes(BTreeMap::from([(other, patch())]), None)
            .expect("another object writes");
    }
}
