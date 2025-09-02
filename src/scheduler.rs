// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rand::{Rng, SeedableRng, rngs::StdRng};
use tokio::{sync::mpsc::UnboundedSender, task::JoinHandle};

use crate::{
    config::{Config, JobDefinition},
    task::{Task, TaskKind},
};

pub fn scheduler(config: &Config, sender: UnboundedSender<Task>) -> Result<JoinHandle<()>> {
    let jobs = config
        .jobs
        .iter()
        .map(|job| (job.id.clone(), job.clone()))
        .collect::<HashMap<String, JobDefinition>>();

    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut rng = StdRng::seed_from_u64(seed);

    let handler = tokio::spawn(async move {
        tracing::info!("Job scheduler started");
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let mut upcoming_jobs: HashMap<String, DateTime<Utc>> = HashMap::new();
        for (id, job) in &jobs {
            let job = job.clone();
            let mut upcoming = job.schedule.upcoming();
            if job.time_spread.as_secs() > 0 {
                upcoming +=
                    Duration::from_secs_f64(rng.random::<f64>() * job.time_spread.as_secs_f64());
            }
            upcoming_jobs.insert(id.clone(), upcoming);
            tracing::info!("Job {} scheduled for {:?}", id, upcoming);
        }
        loop {
            let mut jobs_to_exectute = vec![];
            for (id, date) in upcoming_jobs.iter_mut() {
                if *date <= Utc::now() {
                    jobs_to_exectute.push(id.clone());
                    let job = jobs.get(id).unwrap();
                    let mut upcoming = job.schedule.upcoming();
                    if job.time_spread.as_secs() > 0 {
                        upcoming += Duration::from_secs_f64(
                            rng.random::<f64>() * job.time_spread.as_secs_f64(),
                        );
                    }
                    *date = upcoming;
                    tracing::info!("Job {} scheduled for {:?}", id, upcoming);
                }
            }
            for id in jobs_to_exectute.drain(..) {
                tracing::info!("Executing job {}", id);
                let job = jobs.get(&id).unwrap();
                match job.kind {
                    crate::config::JobKind::MemoryMantainance => {
                        let _ = sender.send(Task::new(crate::task::TaskKind::MemoryMantainance));
                    }
                    crate::config::JobKind::Sleep => {
                        let _ = sender.send(Task::new(TaskKind::Sleep));
                    }
                }
            }
            interval.tick().await;
        }
    });
    Ok(handler)
}
