// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rand::{Rng, SeedableRng, rngs::StdRng};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

use crate::{
    config::{Config, JobDefinition},
    database::DbClient,
    task::{Task, TaskKind},
};

pub fn scheduler(
    config: &Config,
    db_client: DbClient,
    sender: UnboundedSender<Task>,
    upcoming_jobs: Arc<DashMap<String, DateTime<Utc>>>,
    mut activity_listener: UnboundedReceiver<()>,
) -> Result<JoinHandle<()>> {
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
    let mut job_activity = HashSet::<String>::new();

    let handler = tokio::spawn(async move {
        tracing::info!("Job scheduler started");
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        for (id, job) in &jobs {
            let job = job.clone();
            if !job.disable_on_inactivity || job_activity.contains(id) {
                if let Some(mut upcoming) = job.schedule.upcoming() {
                    if job.time_spread.as_secs() > 0 {
                        upcoming += Duration::from_secs_f64(
                            rng.random::<f64>() * job.time_spread.as_secs_f64(),
                        );
                    }
                    upcoming_jobs.insert(id.clone(), upcoming);
                    tracing::info!("[{id}] scheduled for {:?}", upcoming);
                }
            } else {
                tracing::info!("[{id}] not scheduled due inactivity");
            }
        }
        loop {
            let assist_tasks = db_client
                .scheduled_tasks()
                .await
                .into_iter()
                .map(|task| (task.id.to_string(), task))
                .collect::<HashMap<_, _>>();
            for (task_id, task) in &assist_tasks {
                if !upcoming_jobs.contains_key(task_id) {
                    if let Some(upcoming) = task.schedule.upcoming() {
                        upcoming_jobs.insert(task.id.to_string(), upcoming);
                        tracing::info!("[assist_task_{}] scheduled for {:?}", task.id, upcoming);
                    } else {
                        tracing::info!("[assist_task_{}] delete past task", task.id);
                        db_client.delete_scheduled_task(task.id).await.ok();
                    }
                }
            }
            upcoming_jobs.retain(|task_id, _time| {
                assist_tasks.contains_key(task_id) || jobs.contains_key(task_id)
            });

            let mut was_activity = false;
            while activity_listener.try_recv().is_ok() {
                was_activity = true;
            }

            if was_activity {
                for job in jobs.values() {
                    if job.disable_on_inactivity && !job_activity.contains(job.id.as_str()) {
                        let id = job.id.clone();
                        job_activity.insert(id.clone());
                        if !upcoming_jobs.contains_key(&id)
                            && let Some(mut upcoming) = job.schedule.upcoming()
                        {
                            if job.time_spread.as_secs() > 0 {
                                upcoming += Duration::from_secs_f64(
                                    rng.random::<f64>() * job.time_spread.as_secs_f64(),
                                );
                            }
                            upcoming_jobs.insert(id.clone(), upcoming);
                            tracing::info!("[{id}] scheduled for {:?}", upcoming);
                        }
                    }
                }
            }

            let mut jobs_to_exectute = vec![];
            let mut jobs_to_remove = vec![];

            for mut entry in upcoming_jobs.iter_mut() {
                if entry.value() <= &Utc::now() {
                    let id = entry.key().clone();
                    jobs_to_exectute.push(entry.key().clone());
                    if let Some(job) = jobs.get(&id) {
                        if !job.disable_on_inactivity {
                            if let Some(mut upcoming) = job.schedule.upcoming() {
                                if job.time_spread.as_secs() > 0 {
                                    upcoming += Duration::from_secs_f64(
                                        rng.random::<f64>() * job.time_spread.as_secs_f64(),
                                    );
                                }
                                *entry.value_mut() = upcoming;
                                tracing::info!("[{id}] scheduled for {:?}", upcoming);
                            }
                        } else {
                            jobs_to_remove.push(id.clone());
                            tracing::info!("[{id}] not scheduled due inactivity");
                        }
                    } else if let Some(task) = assist_tasks.get(&id)
                        && let Some(upcoming) = task.schedule.upcoming()
                    {
                        *entry.value_mut() = upcoming;
                        tracing::info!("[assist_task_{id}] scheduled for {:?}", upcoming);
                    }
                }
            }

            if !jobs_to_remove.is_empty() {
                upcoming_jobs.retain(|id, _| !jobs_to_remove.contains(id));
            }

            for id in jobs_to_exectute.drain(..) {
                tracing::info!("Executing [{id}]");
                if let Some(job_definition) = jobs.get(&id) {
                    job_activity.remove(&id);
                    match job_definition.kind {
                        crate::config::JobKind::MemoryMantainance => {
                            let _ =
                                sender.send(Task::new(crate::task::TaskKind::MemoryMantainance));
                        }
                        crate::config::JobKind::Sleep => {
                            let _ = sender.send(Task::new(TaskKind::Sleep));
                        }
                    }
                } else if let Some(task) = assist_tasks.get(&id) {
                    let _ = sender.send(Task::new(TaskKind::AssistantTask {
                        sheduled_task_id: task.id,
                        content: task.content.clone(),
                    }));
                }
            }
            interval.tick().await;
        }
    });
    Ok(handler)
}
