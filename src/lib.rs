use octocrab::models::Permissions;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Repo {
    pub name: String,
    pub teams: Vec<Team>,
    pub collaborators: Vec<Collaborator>,
    pub branch_protections: Vec<BranchProtection>,
    pub archived: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BranchProtection {
    pub pattern: String,
    pub status_checks: Vec<String>,
    pub dismiss_stale_review: bool,
    pub pr_required: bool,
    pub required_approvals: i64,
    pub push_allowances: Vec<String>,
    pub restrict_pushes: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Team {
    pub name: String,
    pub permission: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Collaborator {
    pub name: String,
    pub permissions: Permissions,
}
