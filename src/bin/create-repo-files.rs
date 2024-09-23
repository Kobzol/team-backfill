use anyhow::Context;
use indicatif::ProgressIterator;
use octocrab::models::repos::RepoCommit;
use octocrab::models::Repository;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use backfill::Repo;

fn repo_path(repo: &Repo) -> PathBuf {
    let mut parent = PathBuf::from("../repos/rust-lang");
    if repo.archived {
        parent = PathBuf::from("../repos/archive/rust-lang");
    }
    std::fs::create_dir_all(&parent).unwrap();
    parent.join(format!("{}.toml", repo.name))
}

fn is_managed(repo: &Repo) -> bool {
    let path = repo_path(repo);
    Command::new("git")
        .args([
            "ls-files",
            "--error-unmatch",
            path.to_str().unwrap().strip_prefix("../").unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir("../")
        .status()
        .unwrap()
        .success()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct BranchProtectionEntry {
    pattern: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ci_checks: Vec<String>,
    #[serde(skip_serializing_if = "is_false")]
    dismiss_stale_review: bool,
    #[serde(skip_serializing_if = "is_one")]
    required_approvals: i64,
    #[serde(skip_serializing_if = "is_true")]
    pr_required: bool,
    #[serde(skip_serializing_if = "is_false")]
    restrict_pushes: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allowed_merge_teams: Vec<String>,
}

fn is_one(value: &i64) -> bool {
    *value == 1
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct RepoEntry {
    org: String,
    name: String,
    description: String,
    bots: Vec<String>,
    access: AccessEntry,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branch_protections: Vec<BranchProtectionEntry>,
}

#[derive(serde::Serialize)]
struct AccessEntry {
    teams: HashMap<String, String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    individuals: HashMap<String, String>,
}

#[derive(Debug)]
struct EnhancedRepo {
    repo: Repo,
    repository: Repository,
    last_commit: Option<RepoCommit>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = "repos.json";

    let mut repos: Vec<Repo> = serde_json::from_slice(
        &std::fs::read(input).with_context(|| anyhow::anyhow!("Did not find {input}"))?,
    )?;
    repos.sort_by_key(|repo| repo.name.clone());
    repos.dedup_by_key(|repo| repo.name.clone());

    let mut existing = 0;
    let mut written = 0;

    let token =
        std::env::var("GITHUB_TOKEN").context("Missing GITHUB_TOKEN environment variable")?;

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(token.to_string())
        .build()?;

    let mut downloaded_repos = vec![];

    for repo in repos.into_iter().progress() {
        if is_managed(&repo) {
            existing += 1;
            continue;
        }

        // if repo.name == "rust" {
        //     continue;
        // }

        let repo_client = client.repos("rust-lang", &repo.name);
        let repository = match repo_client.get().await {
            Ok(r) => r,
            Err(error) => {
                eprintln!("Error when getting repository {}: {error}", repo.name);
                continue;
            }
        };
        let default_branch = repository
            .default_branch
            .clone()
            .unwrap_or("master".to_string());
        let mut commits = repo_client
            .list_commits()
            .branch(default_branch)
            .since(chrono::Utc::now() - chrono::Duration::days(30 * 6))
            .per_page(50)
            .send()
            .await?
            .take_items();
        // if commits.is_empty() {
        //     println!("{} is inactive", repo.name);
        //     continue;
        // }

        if !repo.teams.is_empty() {
            downloaded_repos.push(EnhancedRepo {
                repo,
                repository,
                last_commit: commits.get(0).cloned(),
            });
        } else {
            println!("{} has no teams", repo.name);
        }

        println!("{}", downloaded_repos.len());
        // if active_repos.len() > 3 {
        //     break;
        // }
    }

    downloaded_repos.sort_by_key(|repo| {
        (
            repo.last_commit
                .as_ref()
                .map(|c| Reverse(c.commit.author.as_ref().unwrap().date.unwrap())),
            repo.repo.name.clone(),
        )
    });

    for repo in downloaded_repos {
        let EnhancedRepo {
            repo,
            repository,
            last_commit,
        } = repo;
        let path = repo_path(&repo);
        let individuals = repo
            .collaborators
            .iter()
            .filter_map(|collaborator| {
                let perm = &collaborator.permissions;
                let permission = if perm.admin {
                    "admin"
                } else if perm.maintain {
                    "maintain"
                } else if perm.push {
                    "write"
                } else if perm.triage {
                    "triage"
                } else {
                    return None;
                };

                Some((collaborator.name.to_string(), permission.to_string()))
            })
            .collect();
        let teams = repo
            .teams
            .iter()
            .map(|team| {
                let permission = match team.permission.as_str() {
                    "push" => "write",
                    s => s,
                };

                (team.name.to_string(), permission.to_string())
            })
            .collect();

        let branch_protections = repo
            .branch_protections
            .iter()
            .map(|protection| {
                let allowed_merge_teams = protection
                    .push_allowances
                    .iter()
                    .filter_map(|allowance| {
                        let allowance: serde_json::Value =
                            serde_json::from_str(&allowance).unwrap();
                        let actor = allowance.get("actor")?.as_object()?;
                        if actor.contains_key("login") {
                            Some(actor.get("login")?.as_str()?.to_string())
                        } else if actor.contains_key("name") {
                            Some(actor.get("name")?.as_str()?.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                BranchProtectionEntry {
                    pattern: protection.pattern.clone(),
                    ci_checks: protection.status_checks.clone(),
                    dismiss_stale_review: protection.dismiss_stale_review,
                    required_approvals: protection.required_approvals,
                    pr_required: protection.pr_required,
                    restrict_pushes: protection.restrict_pushes,
                    allowed_merge_teams,
                }
            })
            .collect();
        let entry = RepoEntry {
            org: "rust-lang".to_string(),
            name: repo.name.clone(),
            description: repository.description.unwrap_or_default(),
            bots: vec![],
            access: AccessEntry { teams, individuals },
            branch_protections,
        };

        println!(
            "Writing {} ({:?})",
            repo.name,
            last_commit.map(|c| c.commit.author.as_ref().unwrap().date)
        );
        let path = format!("{}.tmp", path.display());
        std::fs::write(path, toml::to_string_pretty(&entry)?)?;
        written += 1;
    }

    println!("Written {written} repo(s)");

    Ok(())
}
