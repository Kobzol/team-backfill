use backfill::{BranchProtection, Collaborator, Repo, Team};
use octocrab::models::Repository;
use octocrab::Octocrab;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = "";
    let org = "rust-lang";

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(token.to_string())
        .build()?;
    let mut gh_repos = vec![];
    let mut page = 0u32;
    loop {
        let mut repos = client
            .orgs(org)
            .list_repos()
            .page(page)
            .per_page(100)
            .send()
            .await?;
        for repo in repos.take_items() {
            gh_repos.push(repo);
        }

        if repos.next.is_none() {
            break;
        } else {
            page += 1;
        }
    }

    let mut repositories: Vec<Repo> = vec![];
    for repo in gh_repos {
        repositories.push(handle_repo(repo, &client).await?);
    }

    println!("{}", serde_json::to_string_pretty(&repositories)?);

    Ok(())
}

async fn handle_repo(repo: Repository, client: &Octocrab) -> anyhow::Result<Repo> {
    // Teams
    let mut team_page = 0u32;
    let mut teams = vec![];
    loop {
        let mut team_response = client
            .repos(&repo.owner.as_ref().unwrap().login, &repo.name)
            .list_teams()
            .per_page(100)
            .page(team_page)
            .send()
            .await?;
        for team in team_response.take_items() {
            teams.push(Team {
                name: team.name,
                permission: team.permission,
            });
        }

        if team_response.next.is_none() {
            break;
        } else {
            team_page += 1;
        }
    }

    // Collaborators
    let mut collabs_page = 0u32;
    let mut collaborators = vec![];
    loop {
        let mut collab_response = client
            .repos(&repo.owner.as_ref().unwrap().login, &repo.name)
            .list_collaborators()
            .per_page(100)
            .page(collabs_page)
            .send()
            .await?;
        for collaborator in collab_response.take_items() {
            collaborators.push(Collaborator {
                name: collaborator.author.login,
                permissions: collaborator.permissions,
            });
        }

        if collab_response.next.is_none() {
            break;
        } else {
            collabs_page += 1;
        }
    }

    // Branch protections
    let query = format!(
        r#"query MyQuery {{
    repository(name: "{}", owner: "{}") {{
      branchProtectionRules(first: 10) {{
        edges {{
          node {{
            dismissesStaleReviews
            pattern
            requiredStatusChecks {{
              context
            }}
            requiresApprovingReviews
            requiredApprovingReviewCount
            restrictsPushes
            pushAllowances(first:100) {{
                nodes {{
                    id
                    actor {{
                        __typename
                        ... on User {{
                            login
                        }}
                        ... on Team {{
                            name
                        }}
                    }}
                }}
            }}
          }}
        }}
      }}
    }}
    }}"#,
        repo.name,
        repo.owner.unwrap().login
    );
    let request = serde_json::json!({ "query": query });
    let result: serde_json::Value = client.graphql(&request).await?;
    let branch_protections: Vec<_> = result
        .get("data")
        .unwrap()
        .get("repository")
        .unwrap()
        .get("branchProtectionRules")
        .unwrap()
        .get("edges")
        .unwrap()
        .as_array()
        .unwrap()
        .into_iter()
        .map(|value| {
            let obj = value.get("node").unwrap();
            BranchProtection {
                pattern: obj.get("pattern").unwrap().as_str().unwrap().to_string(),
                status_checks: obj
                    .get("requiredStatusChecks")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .into_iter()
                    .map(|s| s.get("context").unwrap().as_str().unwrap().to_string())
                    .collect(),
                dismiss_stale_review: obj.get("dismissesStaleReviews").unwrap().as_bool().unwrap(),
                pr_required: obj
                    .get("requiresApprovingReviews")
                    .unwrap()
                    .as_bool()
                    .unwrap(),
                required_approvals: obj
                    .get("requiredApprovingReviewCount")
                    .unwrap()
                    .as_i64()
                    .unwrap_or(0),
                push_allowances: obj
                    .get("pushAllowances")
                    .and_then(|v| v.as_object())
                    .and_then(|v| v.get("nodes"))
                    .and_then(|v| v.as_array())
                    .map(|v| v.iter().map(|t| t.to_string()).collect())
                    .unwrap_or_default(),
                restrict_pushes: obj
                    .get("restrictsPushes")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        })
        .collect();

    Ok(Repo {
        name: repo.name,
        archived: repo.archived.unwrap_or(false),
        teams,
        collaborators,
        branch_protections,
    })
}
