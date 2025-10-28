// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime},
};

use anyhow::Result;
use chrono::Utc;
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

pub async fn scheduler(
    config: &Config,
    db_client: DbClient,
    sender: UnboundedSender<Task>,
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
    let mut upcoming_jobs = db_client
        .get_scheduler()
        .await?
        .into_iter()
        .collect::<HashMap<_, _>>();

    let handler = tokio::spawn(async move {
        tracing::info!("Job scheduler started");
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        for (id, job) in &jobs {
            let job = job.clone();
            if let Some(upcoming) = upcoming_jobs.get(id) {
                tracing::info!("[{id}] scheduled for {:?}", upcoming);
            } else if !job.disable_on_inactivity || job_activity.contains(id) {
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
        db_client
            .update_scheduler(upcoming_jobs.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .await
            .ok();
        loop {
            let mut changed = false;
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
                        changed = true;
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
                            changed = true;
                            tracing::info!("[{id}] scheduled for {:?}", upcoming);
                        }
                    }
                }
            }

            let mut jobs_to_exectute = vec![];
            let mut jobs_to_remove = vec![];

            for (key, value) in upcoming_jobs.iter_mut() {
                if *value <= Utc::now() {
                    let id = key.clone();
                    jobs_to_exectute.push(key.clone());
                    if let Some(job) = jobs.get(&id) {
                        if !job.disable_on_inactivity {
                            if let Some(mut upcoming) = job.schedule.upcoming() {
                                if job.time_spread.as_secs() > 0 {
                                    upcoming += Duration::from_secs_f64(
                                        rng.random::<f64>() * job.time_spread.as_secs_f64(),
                                    );
                                }
                                *value = upcoming;
                                changed = true;
                                tracing::info!("[{id}] scheduled for {:?}", upcoming);
                            }
                        } else {
                            jobs_to_remove.push(id.clone());
                            changed = true;
                            tracing::info!("[{id}] not scheduled due inactivity");
                        }
                    } else if let Some(task) = assist_tasks.get(&id)
                        && let Some(upcoming) = task.schedule.upcoming()
                    {
                        *value = upcoming;
                        changed = true;
                        tracing::info!("[assist_task_{id}] scheduled for {:?}", upcoming);
                    }
                }
            }

            if !jobs_to_remove.is_empty() {
                upcoming_jobs.retain(|id, _| !jobs_to_remove.contains(id));
            }

            if changed {
                db_client
                    .update_scheduler(upcoming_jobs.iter().map(|(k, v)| (k.clone(), *v)).collect())
                    .await
                    .ok();
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
