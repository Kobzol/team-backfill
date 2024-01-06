use chrono::FixedOffset;
use octocrab::models::commits::Commit;
use octocrab::models::repos::RepoCommit;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::Sub;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use toml::to_string;

use backfill::Repo;

fn path(repo: &Repo) -> PathBuf {
    PathBuf::from("../repos/rust-lang").join(format!("{}.toml", repo.name))
}

fn is_managed(repo: &Repo) -> bool {
    let path = path(repo);
    Command::new("git")
        .args(["ls-files", "--error-unmatch", path.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[derive(serde::Serialize)]
struct RepoEntry {
    org: String,
    name: String,
    description: String,
    bots: Vec<String>,
    access: AccessEntry,
}

#[derive(serde::Serialize)]
struct AccessEntry {
    teams: HashMap<String, String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    individuals: HashMap<String, String>,
}

#[derive(Debug)]
struct ActiveRepo {
    repo: Repo,
    last_commit: RepoCommit,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = "repos.json";

    let mut repos: Vec<Repo> = serde_json::from_slice(&std::fs::read(input)?)?;
    repos.sort_by_key(|repo| repo.name.clone());
    repos.dedup_by_key(|repo| repo.name.clone());

    let mut existing = 0;
    let mut written = 0;

    let token = "";

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(token.to_string())
        .build()?;

    let mut active_repos = vec![];

    for repo in repos {
        if is_managed(&repo) {
            existing += 1;
            continue;
        }
        if repo.name == "rust" {
            continue;
        }

        let repo_client = client.repos("rust-lang", &repo.name);
        let Ok(repository) = repo_client.get().await else {
            continue;
        };
        if repository.archived.unwrap_or(false) {
            continue;
        }
        let default_branch = repository.default_branch.unwrap_or("master".to_string());
        let commits = repo_client
            .list_commits()
            .branch(default_branch)
            .since(chrono::Utc::now() - chrono::Duration::days(30 * 6))
            .per_page(50)
            .send()
            .await?
            .take_items();
        if commits.is_empty() {
            println!("{} is inactive", repo.name);
            continue;
        }

        if !repo.teams.is_empty() {
            active_repos.push(ActiveRepo {
                repo,
                last_commit: commits[0].clone(),
            });
        } else {
            println!("{} has no teams", repo.name);
        }

        // if active_repos.len() > 3 {
        //     break;
        // }
    }

    active_repos.sort_by_key(|repo| {
        Reverse(
            repo.last_commit
                .commit
                .author
                .as_ref()
                .unwrap()
                .date
                .unwrap(),
        )
    });

    for repo in active_repos {
        let ActiveRepo { repo, last_commit } = repo;
        let path = path(&repo);
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

        let entry = RepoEntry {
            org: "rust-lang".to_string(),
            name: repo.name.clone(),
            description: "".to_string(),
            bots: vec![],
            access: AccessEntry { teams, individuals },
        };

        println!(
            "Writing {} ({:?})",
            repo.name,
            last_commit.commit.author.as_ref().unwrap().date
        );
        let path = format!("{}.tmp", path.display());
        std::fs::write(path, toml::to_string_pretty(&entry)?)?;
        written += 1;
    }

    println!("Written {written} repo(s)");

    Ok(())
}
