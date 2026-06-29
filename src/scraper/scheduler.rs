use crate::config::DsearchConfig;
use crate::storage::Store;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

/// Start the periodic scraper job scheduler.
/// Spawns a background task that runs each configured scraper job
/// at its specified interval. Jobs with `refresh = "once"` are run
/// once on startup and then disabled. Jobs with `refresh = "interval"`
/// are run every `interval_secs`. Jobs with `refresh = "on-change"`
/// are run at `interval_secs` but only store results if the content changed.
pub fn start_scheduler(
    store: Arc<Store>,
    config: DsearchConfig,
    data_dir: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_scheduler_loop(store, config, data_dir).await;
    })
}

async fn run_scheduler_loop(
    store: Arc<Store>,
    initial_config: DsearchConfig,
    data_dir: std::path::PathBuf,
) {
    let mut job_states: Vec<JobState> = initial_config
        .scraper
        .jobs
        .iter()
        .map(|j| JobState {
            name: j.name.clone(),
            source: j.source.as_str().to_string(),
            target: j.target.clone(),
            refresh: j.refresh.as_str().to_string(),
            interval_secs: j.interval_secs,
            lifecycle: j.lifecycle.as_str().to_string(),
            ttl_secs: j.ttl_secs,
            last_run: 0,
            run_count: 0,
            completed: false,
        })
        .collect();

    info!("Scraper scheduler started with {} job(s)", job_states.len());

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        // Reload config to pick up new/changed jobs
        if let Ok(new_config) = crate::config::load_config(&data_dir) {
            sync_job_states(&mut job_states, &new_config);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for job in &mut job_states {
            if job.completed {
                continue;
            }

            let should_run = match job.refresh.as_str() {
                "once" => job.run_count == 0,
                "interval" | "on-change" => now >= job.last_run + job.interval_secs,
                _ => false,
            };

            if !should_run {
                continue;
            }

            debug!("Running scraper job '{}' ({})", job.name, job.source);
            let result = run_job(&store, job).await;
            match result {
                Ok(record_id) => {
                    job.run_count += 1;
                    job.last_run = now;
                    if job.refresh == "once" {
                        job.completed = true;
                    }
                    info!(
                        "Scraper job '{}' completed: record {} (run #{})",
                        job.name, record_id, job.run_count
                    );
                }
                Err(e) => {
                    error!("Scraper job '{}' failed: {}", job.name, e);
                    job.last_run = now; // Don't retry immediately on failure
                }
            }
        }
    }
}

struct JobState {
    name: String,
    source: String,
    target: String,
    refresh: String,
    interval_secs: u64,
    lifecycle: String,
    ttl_secs: u64,
    last_run: u64,
    run_count: u64,
    completed: bool,
}

async fn run_job(store: &Store, job: &JobState) -> Result<String, String> {
    match job.source.as_str() {
        "url" => {
            let result = crate::scraper::job::run_url_job(
                store,
                &job.name,
                &job.target,
                &job.lifecycle,
                job.ttl_secs,
            )
            .await?;
            Ok(result.record_id)
        }
        "feed" => {
            let result = crate::scraper::discovery::html_scrape::run_feed_job(
                store,
                &job.name,
                &job.target,
                job.ttl_secs,
            )
            .await?;
            Ok(format!(
                "{}: {} entries, {} inserted, {} replaced, {} skipped",
                result.job_name, result.total, result.inserted, result.replaced, result.skipped
            ))
        }
        "api" => {
            let result = crate::scraper::discovery::api::run_api_job(
                store,
                &job.name,
                &job.target,
                job.ttl_secs,
            )
            .await?;
            Ok(format!(
                "{}: {} items, {} inserted, {} replaced, {} skipped",
                result.job_name, result.total, result.inserted, result.replaced, result.skipped
            ))
        }
        "keyword" => {
            // Keyword discovery: resolve keyword to URLs via search providers, then scrape each
            let providers = crate::scraper::discovery::providers::load_providers(
                &std::path::PathBuf::from(&job.target),
            );
            let urls = crate::scraper::discovery::providers::resolve_keyword_to_urls(
                &providers, &job.name,
            );
            if urls.is_empty() {
                return Err(format!(
                    "No search providers found for keyword '{}'",
                    job.name
                ));
            }
            let mut total_inserted = 0usize;
            for (provider_name, url) in &urls {
                match crate::scraper::job::run_url_job(
                    store,
                    provider_name,
                    url,
                    &job.lifecycle,
                    job.ttl_secs,
                )
                .await
                {
                    Ok(r) => total_inserted += if r.inserted { 1 } else { 0 },
                    Err(e) => {
                        tracing::warn!(
                            "Keyword scrape '{}' from {} failed: {}",
                            job.name,
                            provider_name,
                            e
                        );
                    }
                }
            }
            Ok(format!(
                "keyword '{}': {} URLs scraped, {} inserted",
                job.name,
                urls.len(),
                total_inserted
            ))
        }
        _ => Err(format!("Unknown source type: {}", job.source)),
    }
}

/// Sync job states with the current config, adding new jobs and
/// removing ones that no longer exist.
fn sync_job_states(states: &mut Vec<JobState>, config: &DsearchConfig) {
    // Add new jobs
    for job in &config.scraper.jobs {
        if !states.iter().any(|s| s.name == job.name) {
            states.push(JobState {
                name: job.name.clone(),
                source: job.source.as_str().to_string(),
                target: job.target.clone(),
                refresh: job.refresh.as_str().to_string(),
                interval_secs: job.interval_secs,
                lifecycle: job.lifecycle.as_str().to_string(),
                ttl_secs: job.ttl_secs,
                last_run: 0,
                run_count: 0,
                completed: false,
            });
        }
    }

    // Remove jobs that no longer exist in config
    let job_names: Vec<String> = config.scraper.jobs.iter().map(|j| j.name.clone()).collect();
    states.retain(|s| job_names.contains(&s.name));

    // Update existing jobs' parameters
    for state in states.iter_mut() {
        if let Some(job) = config.scraper.jobs.iter().find(|j| j.name == state.name) {
            state.target = job.target.clone();
            state.interval_secs = job.interval_secs;
            state.ttl_secs = job.ttl_secs;
            state.refresh = job.refresh.as_str().to_string();
            state.lifecycle = job.lifecycle.as_str().to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_job_states_adds_new() {
        let mut states = vec![JobState {
            name: "old".to_string(),
            source: "url".to_string(),
            target: "https://old.example.com".to_string(),
            refresh: "once".to_string(),
            interval_secs: 3600,
            lifecycle: "ephemeral".to_string(),
            ttl_secs: 3600,
            last_run: 0,
            run_count: 1,
            completed: true,
        }];

        let mut config = DsearchConfig::default();
        config.scraper.jobs.push(crate::model::ScrapeJob {
            name: "old".to_string(),
            source: crate::model::ScrapeSource::Url,
            target: "https://old.example.com".to_string(),
            transform: None,
            refresh: crate::model::RefreshPolicy::Once,
            interval_secs: 3600,
            lifecycle: crate::model::Lifecycle::Ephemeral,
            ttl_secs: 3600,
            max_results: None,
        });
        config.scraper.jobs.push(crate::model::ScrapeJob {
            name: "new".to_string(),
            source: crate::model::ScrapeSource::Url,
            target: "https://new.example.com".to_string(),
            transform: None,
            refresh: crate::model::RefreshPolicy::Interval,
            interval_secs: 1800,
            lifecycle: crate::model::Lifecycle::Ephemeral,
            ttl_secs: 3600,
            max_results: None,
        });

        sync_job_states(&mut states, &config);

        assert_eq!(states.len(), 2);
        assert!(states.iter().any(|s| s.name == "new"));
        // Old job should keep its run_count
        assert_eq!(
            states.iter().find(|s| s.name == "old").unwrap().run_count,
            1
        );
    }

    #[test]
    fn test_sync_job_states_removes_deleted() {
        let mut states = vec![JobState {
            name: "removed".to_string(),
            source: "url".to_string(),
            target: "https://removed.example.com".to_string(),
            refresh: "once".to_string(),
            interval_secs: 3600,
            lifecycle: "ephemeral".to_string(),
            ttl_secs: 3600,
            last_run: 0,
            run_count: 0,
            completed: false,
        }];

        let config = DsearchConfig::default(); // No jobs

        sync_job_states(&mut states, &config);
        assert!(states.is_empty());
    }
}
