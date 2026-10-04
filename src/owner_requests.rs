//! Owner decisions prompted by Claude Code's generated ask rules.
use crate::{
    paths::Ctx,
    project::{self, Project},
};
use anyhow::{Context, Result, ensure};

pub fn decide(ctx: &Ctx, slug: &str, id: &str, summary: &str, approve: bool) -> Result<()> {
    project::validate_slug(slug)?;
    let project = Project {
        root: ctx.root.clone(),
        slug: slug.into(),
    };
    ensure!(project.project_md().is_file(), "project {slug} not found");
    if !project.dir().join(".state/format.json").exists() {
        return crate::worker_permissions::decide_ask(ctx, slug, id, summary, approve);
    }
    // Serialize repository edits and decisions across concurrent owner commands.
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(project.state_dir().join("owner-request.lock"))?;
    lock.lock()?;
    let mut store = herdr_farm::migration::open_active(&project.dir())?;
    let request = store.owner_request(id)?.context("owner request missing")?;
    let now = jiff::Timestamp::now().as_millisecond();
    ensure!(
        request.summary == summary,
        "--summary must be byte-identical to the stored summary"
    );
    ensure!(request.status == "pending", "owner request already decided");
    ensure!(!approve || now < request.expires, "owner request expired");
    if approve && request.action == "repository" {
        let text = String::from_utf8(herdr_farm::migration::read_plan_file(
            &project.project_md(),
        )?)?;
        let rest = text
            .strip_prefix("+++\n")
            .context("PROJECT.md header missing")?;
        let (front, body) = rest
            .split_once("\n+++\n")
            .or_else(|| rest.strip_suffix("\n+++").map(|front| (front, "")))
            .context("PROJECT.md header unclosed")?;
        let mut settings: toml::Value = toml::from_str(front)?;
        let table = settings
            .as_table_mut()
            .context("PROJECT.md settings must be a table")?;
        let repos = table
            .entry("repos")
            .or_insert_with(|| toml::Value::Array(vec![]))
            .as_array_mut()
            .context("repos must be an array")?;
        if !repos.iter().any(|r| {
            r.get("machine").is_none()
                && r.get("path").and_then(|v| v.as_str()) == Some(&request.repository)
        }) {
            repos.push(toml::Value::try_from(project::Repo {
                path: request.repository.clone(),
                machine: None,
            })?);
            project::write_atomic(
                &project.project_md(),
                format!("+++\n{}+++\n{body}", toml::to_string(&settings)?).as_bytes(),
            )?;
        }
    }
    store.decide_owner_request(id, summary, approve, now)?;
    println!("{} {id}", if approve { "approved" } else { "rejected" });
    Ok(())
}
