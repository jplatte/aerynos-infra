use std::{convert::Infallible, future::Future, time::Duration};

use color_eyre::{Result, eyre::Context};
use service::{Collectable, endpoint};
use tokio::{
    sync::mpsc,
    time::{self, Instant},
};
use tracing::{Span, debug, error, info};

use crate::{Manager, repository, task};

const TIMER_INTERVAL: Duration = Duration::from_secs(30);

pub type Sender = mpsc::UnboundedSender<Message>;

#[derive(Debug, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum Message {
    AllocateBuilds,
    BuildSucceeded {
        task_id: task::Id,
        builder: endpoint::Id,
        collectables: Vec<Collectable>,
    },
    BuildFailed {
        task_id: task::Id,
        builder: endpoint::Id,
        collectables: Vec<Collectable>,
    },
    ImportSucceeded {
        task_id: task::Id,
    },
    ImportFailed {
        task_id: task::Id,
    },
    Timer(Instant),
}

pub async fn run(mut manager: Manager) -> Result<(Sender, impl Future<Output = Result<(), Infallible>> + use<>)> {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Message>();

    let task = {
        let sender = sender.clone();

        tokio::spawn(timer_task(sender.clone()));

        async move {
            while let Some(message) = receiver.recv().await {
                let kind = message.to_string();

                if let Err(e) = handle_message(&sender, &mut manager, message).await {
                    let error = service::error::chain(e.as_ref() as &dyn std::error::Error);
                    error!(message = kind, %error, "Error handling message");
                }
            }

            info!("Worker exiting");

            Ok(())
        }
    };

    let _ = sender.send(Message::AllocateBuilds);

    Ok((sender, task))
}

async fn timer_task(sender: Sender) -> Result<(), Infallible> {
    let mut interval = time::interval(TIMER_INTERVAL);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        let _ = sender.send(Message::Timer(interval.tick().await));
    }
}

async fn handle_message(sender: &Sender, manager: &mut Manager, message: Message) -> Result<()> {
    match message {
        Message::AllocateBuilds => allocate_builds(manager).await,
        Message::BuildSucceeded {
            task_id,
            builder,
            collectables,
        } => build_succeeded(sender, manager, task_id, builder, collectables).await,
        Message::BuildFailed {
            task_id,
            builder,
            collectables,
        } => build_failed(sender, manager, task_id, builder, collectables).await,
        Message::ImportSucceeded { task_id } => import_succeeded(sender, manager, task_id).await,
        Message::ImportFailed { task_id } => import_failed(sender, manager, task_id).await,
        Message::Timer(_) => timer(sender, manager).await,
    }
}

#[tracing::instrument(skip_all)]
async fn allocate_builds(manager: &mut Manager) -> Result<()> {
    debug!("Allocating builds");
    manager.allocate_builds().await.context("allocate builds")
}

#[tracing::instrument(skip_all)]
async fn build_succeeded(
    sender: &Sender,
    manager: &mut Manager,
    task_id: task::Id,
    builder: endpoint::Id,
    collectables: Vec<Collectable>,
) -> Result<()> {
    debug!("Build succeeded");

    let publishing_failed = manager
        .build_succeeded(task_id, builder, collectables)
        .await
        .context("manager build succeeded")?;

    // Lifecycle will not continue since task is now failed
    // so drive forward new tasks
    if publishing_failed {
        let _ = sender.send(Message::AllocateBuilds);
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn build_failed(
    sender: &Sender,
    manager: &mut Manager,
    task_id: task::Id,
    builder: endpoint::Id,
    collectables: Vec<Collectable>,
) -> Result<()> {
    debug!("Build failed");

    manager
        .build_failed(task_id, builder, collectables)
        .await
        .context("manager build failed")?;

    let _ = sender.send(Message::AllocateBuilds);

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn import_succeeded(sender: &Sender, manager: &mut Manager, task_id: task::Id) -> Result<()> {
    debug!("Import succeeded");

    manager
        .import_succeeded(task_id)
        .await
        .context("manager import failed")?;

    let _ = sender.send(Message::AllocateBuilds);

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn import_failed(sender: &Sender, manager: &mut Manager, task_id: task::Id) -> Result<()> {
    debug!("Import failed");

    manager.import_failed(task_id).await.context("manager import failed")?;

    let _ = sender.send(Message::AllocateBuilds);

    Ok(())
}

#[tracing::instrument(skip_all, fields(project, repository))]
async fn timer(sender: &Sender, manager: &Manager) -> Result<()> {
    debug!("Timer triggered");

    let span = Span::current();

    let mut have_changes = false;

    let mut conn = manager.acquire().await.context("acquire db conn")?;

    for project in manager.projects().await.context("list projects")? {
        span.record("project", &project.slug);

        for repo in &project.repositories {
            span.record("repository", &repo.name);

            let (mut repo, changed) = repository::refresh(&mut conn, &manager.state, repo.clone())
                .await
                .context("refresh repository")?;

            if changed {
                let repo_db = manager.repository_db(&repo.id)?.clone();

                repo = repository::reindex(&mut conn, &manager.state, repo, repo_db.clone())
                    .await
                    .context("reindex repository")?;

                task::create_missing(&mut conn, manager, &project, &repo, &repo_db)
                    .await
                    .context("create missing tasks")?;

                have_changes = true;
            }
        }
    }

    if have_changes {
        let _ = sender.send(Message::AllocateBuilds);
    }

    Ok(())
}
