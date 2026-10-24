use super::*;
fn token() -> Result<String> {
    std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .map_err(|_| err("adapter_publication_credential_missing"))
}
async fn gh(req: &Request, parts: Vec<String>, input: Option<&Value>) -> Result<Value> {
    let bytes = input.map(|v| v.to_string().into_bytes());
    let out = command(
        &req.adapter_policy.gh,
        &parts,
        bytes.as_deref(),
        &[("GH_TOKEN", token()?)],
        Duration::from_secs(30),
        1048576,
    )
    .await?;
    serde_json::from_slice(&out).map_err(|_| err("adapter_github_response_invalid"))
}
async fn api(req: &Request, path: &str, input: Option<&Value>) -> Result<Value> {
    let mut a = args(&["api", "--hostname", "github.com", path]);
    if input.is_some() {
        a.extend(args(&["--input", "-"]));
    }
    gh(req, a, input).await
}
async fn repository(req: &Request) -> Result<Value> {
    let value = api(req, &format!("repos/{}", req.repository), None).await?;
    if value["full_name"] != req.repository
        || (value["private"] != true && req.adapter_policy.native_agent.is_none())
        || value["permissions"]["push"] != true
        || value["id"].as_u64().is_none()
        || !value["default_branch"].as_str().is_some_and(valid_branch)
    {
        return Err(err("adapter_repository_authority_invalid"));
    }
    Ok(value)
}
fn valid_branch(v: &str) -> bool {
    !v.is_empty()
        && v.len() < 128
        && !v.starts_with('-')
        && !v.contains("..")
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_/.".contains(&b))
}
pub(super) async fn authenticated_clone(req: &Request, destination: &Path) -> Result<()> {
    authenticated_git(
        req,
        args(&[
            "clone",
            "--no-checkout",
            "--",
            &format!("https://github.com/{}.git", req.repository),
            destination
                .to_str()
                .ok_or_else(|| err("adapter_request_invalid"))?,
        ]),
    )
    .await?;
    git_at_clone(req, destination).await
}
async fn git_at_clone(req: &Request, destination: &Path) -> Result<()> {
    let args = vec![
        "-c".into(),
        "core.hooksPath=/dev/null".into(),
        "-C".into(),
        destination.to_string_lossy().into(),
        "checkout".into(),
        "--detach".into(),
        "HEAD".into(),
    ];
    command(
        &req.adapter_policy.git,
        &args,
        None,
        &[],
        Duration::from_secs(30),
        1048576,
    )
    .await?;
    Ok(())
}
async fn authenticated_git(req: &Request, parts: Vec<String>) -> Result<String> {
    let mut a = args(&[
        "-c",
        "credential.helper=",
        "-c",
        "credential.useHttpPath=true",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "protocol.file.allow=never",
    ]);
    a.extend(parts);
    let exe = std::env::current_exe().map_err(|_| err("adapter_command_failed"))?;
    let out = command(
        &req.adapter_policy.git,
        &a,
        None,
        &[
            ("GH_TOKEN", token()?),
            ("GIT_ASKPASS", exe.to_string_lossy().into()),
            ("WORKBOARD_GIT_REPOSITORY", req.repository.clone()),
        ],
        Duration::from_secs(30),
        1048576,
    )
    .await?;
    String::from_utf8(out).map_err(|_| err("adapter_command_invalid"))
}
fn prepared(req: &Request) -> Result<Value> {
    let value = read_value(
        &req.adapter_policy
            .state_root
            .join(&req.run_id)
            .join("prepared.json"),
    )?;
    if value["repository"] != req.repository {
        return Err(err("adapter_repository_authority_invalid"));
    }
    Ok(value)
}
async fn checked_repository(req: &Request) -> Result<Value> {
    let old = prepared(req)?;
    let current = repository(req).await?;
    if current["id"] != old["repository_id"] || current["default_branch"] != old["default_branch"] {
        return Err(err("adapter_repository_authority_changed"));
    }
    Ok(old)
}
async fn pull(req: &Request) -> Result<Option<Value>> {
    let owner = req.repository.split('/').next().unwrap();
    let branch = format!("workboard/{}", req.run_id);
    let result = api(
        req,
        &format!(
            "repos/{}/pulls?head={owner}:{branch}&state=all&per_page=100",
            req.repository
        ),
        None,
    )
    .await?;
    let values = result
        .as_array()
        .ok_or_else(|| err("adapter_github_response_invalid"))?;
    if values.len() > 1 {
        return Err(err("adapter_pull_request_ambiguous"));
    }
    Ok(values.first().cloned())
}
fn validate_pull(req: &Request, p: &Value, base: &Value) -> Result<()> {
    if p["head"]["repo"]["full_name"] != req.repository
        || p["base"]["repo"]["full_name"] != req.repository
        || p["head"]["ref"] != format!("workboard/{}", req.run_id)
        || p["head"]["sha"].as_str() != req.head_sha.as_deref()
        || p["base"]["ref"] != base["default_branch"]
        || p["number"].as_u64().is_none()
        || p["html_url"].as_str()
            != Some(&format!(
                "https://github.com/{}/pull/{}",
                req.repository,
                p["number"].as_u64().unwrap_or(0)
            ))
    {
        return Err(err("adapter_pull_request_changed"));
    }
    Ok(())
}

fn effect(req: &Request, phase: &str, ack: bool) -> Result<()> {
    let path = run_dir(req)?.join(format!(
        "publication-{phase}{}.json",
        if ack { "-ack" } else { "" }
    ));
    write_new(
        &path,
        &json!({"action_id":req.action_id,"run_id":req.run_id,"repository":req.repository,"head_sha":req.head_sha,"base_sha":req.base_sha,"phase":phase,"acknowledged":ack,"at_ms":now_ms()}),
    )
}
fn pending(req: &Request, phase: &str) -> bool {
    let root = req.adapter_policy.state_root.join(&req.run_id);
    root.join(format!("publication-{phase}.json")).exists()
        && !root.join(format!("publication-{phase}-ack.json")).exists()
}
async fn observe_effects(req: &Request, pull: Option<&Value>) -> Result<()> {
    if pending(req, "push") {
        let branch = api(
            req,
            &format!(
                "repos/{}/git/ref/heads/workboard/{}",
                req.repository, req.run_id
            ),
            None,
        )
        .await?;
        if branch["object"]["sha"].as_str() != req.head_sha.as_deref() {
            return Err(err("adapter_publication_uncertain"));
        }
    }
    if pending(req, "pr") && pull.is_none() {
        return Err(err("adapter_publication_uncertain"));
    }
    Ok(())
}
async fn observe_pull(req: &Request, p: &Value, base: &Value) -> Result<Receipt> {
    validate_pull(req, p, base)?;
    let number = p["number"].as_u64().unwrap();
    let current = api(
        req,
        &format!("repos/{}/pulls/{number}", req.repository),
        None,
    )
    .await?;
    validate_pull(req, &current, base)?;
    if pending(req, "merge") && current["merged"] != true && current["state"] != "closed" {
        return Err(err("adapter_publication_uncertain"));
    }
    let mut receipt = req.receipt();
    receipt.pull_request_url = current["html_url"].as_str().map(String::from);
    receipt.merged = Some(current["merged"] == true);
    if current["merged"] == true {
        let merge = current["merge_commit_sha"]
            .as_str()
            .filter(|v| crate::execution_config::sha(v))
            .ok_or_else(|| err("adapter_merge_unverified"))?;
        let workspace = req
            .workspace
            .to_str()
            .ok_or_else(|| err("adapter_request_invalid"))?;
        authenticated_git(
            req,
            args(&[
                "-C",
                workspace,
                "fetch",
                "--no-tags",
                "origin",
                &format!(
                    "+refs/heads/{}:refs/remotes/origin/{}",
                    base["default_branch"].as_str().unwrap(),
                    base["default_branch"].as_str().unwrap()
                ),
            ]),
        )
        .await?;
        let parents = git(req, &["rev-list", "--parents", "-n", "1", merge], None).await?;
        let parents: Vec<_> = parents.split_whitespace().collect();
        if parents.len() != 3
            || parents[1] != req.base_sha.as_deref().unwrap_or("")
            || parents[2] != req.head_sha.as_deref().unwrap_or("")
        {
            return Err(err("adapter_merge_unverified"));
        }
        receipt.merge_commit_sha = Some(merge.into());
    }
    Ok(receipt)
}
async fn merge_gates(req: &Request, pull: &Value, base: &Value) -> Result<()> {
    let number = pull["number"]
        .as_u64()
        .ok_or_else(|| err("adapter_pull_request_changed"))?;
    let endpoint = format!("repos/{}/pulls/{number}", req.repository);
    let mut current = api(req, &endpoint, None).await?;
    let poll_deadline = (now_ms() + 15000).min(req.stage_deadline_ms);
    while current["mergeable"].is_null() && now_ms() < poll_deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        current = api(req, &endpoint, None).await?;
    }
    validate_pull(req, &current, base)?;
    if current["mergeable"] != true || current["draft"] != false || current["state"] != "open" {
        return Err(err("adapter_merge_veto"));
    }
    let reviews = api(
        req,
        &format!(
            "repos/{}/pulls/{number}/reviews?per_page=100",
            req.repository
        ),
        None,
    )
    .await?;
    let reviews = reviews
        .as_array()
        .filter(|a| a.len() < 100)
        .ok_or_else(|| err("adapter_review_unknown"))?;
    let mut decisions = std::collections::HashMap::<String, (u64, String)>::new();
    for review in reviews {
        let state = review["state"]
            .as_str()
            .ok_or_else(|| err("adapter_review_unknown"))?;
        if !["APPROVED", "CHANGES_REQUESTED", "DISMISSED"].contains(&state) {
            continue;
        }
        let id = review["id"]
            .as_u64()
            .ok_or_else(|| err("adapter_review_unknown"))?;
        let login = review["user"]["login"]
            .as_str()
            .ok_or_else(|| err("adapter_review_unknown"))?;
        if decisions.get(login).is_none_or(|(prior, _)| id > *prior) {
            decisions.insert(login.into(), (id, state.into()));
        }
    }
    if decisions
        .values()
        .any(|(_, state)| state == "CHANGES_REQUESTED")
    {
        return Err(err("adapter_review_veto"));
    }
    let (owner, name) = req.repository.split_once('/').unwrap();
    let query = json!({"query":"query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100){nodes{isResolved}pageInfo{hasNextPage}}}}}","variables":{"owner":owner,"name":name,"number":number}});
    let facts = gh(
        req,
        args(&["api", "--hostname", "github.com", "graphql", "--input", "-"]),
        Some(&query),
    )
    .await?;
    let threads = &facts["data"]["repository"]["pullRequest"]["reviewThreads"];
    if facts["errors"].is_array()
        || threads["pageInfo"]["hasNextPage"] != false
        || !threads["nodes"].as_array().is_some_and(|nodes| {
            nodes.len() <= 100 && nodes.iter().all(|n| n["isResolved"] == true)
        })
    {
        return Err(err("adapter_review_veto"));
    }
    Ok(())
}
pub(super) async fn run(req: &Request, _action_dir: &Path) -> Result<Receipt> {
    if !["prepare", "deliver", "observe"].contains(&req.action.as_str()) {
        return Err(err("adapter_action_invalid"));
    }
    // A missing/failed Docker observation is never evidence that an orphaned worker stopped.
    if req.adapter_policy.native_agent.is_some() {
        native::ensure_stopped(req).await?;
    } else if !containers(req).await?.is_empty() {
        return Err(err("adapter_cleanup_unknown"));
    }
    if req.action == "prepare" {
        if req.workspace.exists() {
            return Err(err("adapter_workspace_exists"));
        }
        let repo = repository(req).await?;
        if !req.task_snapshot["project_id"].is_null()
            && (req.task_snapshot["project_repository_id"] != repo["id"]
                || req.task_snapshot["project_default_branch"] != repo["default_branch"])
        {
            return Err(err("adapter_repository_authority_changed"));
        }
        let branch = repo["default_branch"].as_str().unwrap();
        let record =
            json!({"repository":req.repository,"repository_id":repo["id"],"default_branch":branch});
        write_new(&run_dir(req)?.join("prepared.json"), &record)?;
        if req.adapter_policy.native_agent.is_some() {
            native::resolve_workspace(req, &repo).await?;
        }
        let url = format!("https://github.com/{}.git", req.repository);
        authenticated_git(
            req,
            args(&[
                "clone",
                "--no-checkout",
                "--",
                &url,
                req.workspace.to_str().unwrap(),
            ]),
        )
        .await?;
        let base_ref = format!("refs/remotes/origin/{branch}");
        git(req, &["checkout", "--detach", &base_ref], None).await?;
        if req.adapter_policy.native_agent.is_none() {
            readable_source(&req.workspace)?;
        }
        git(req, &["update-index", "--refresh"], None).await?;
        let base = git(req, &["rev-parse", "HEAD"], None).await?;
        let mut receipt = req.receipt();
        receipt.base_ref = Some(base_ref);
        receipt.base_sha = Some(base.trim().into());
        receipt.head_sha = receipt.base_sha.clone();
        return Ok(receipt);
    }
    let base = checked_repository(req).await?;
    if req.action == "observe" {
        let p = pull(req).await?;
        observe_effects(req, p.as_ref()).await?;
        if let Some(p) = p {
            return observe_pull(req, &p, &base).await;
        }
        if pending(req, "merge") {
            return Err(err("adapter_publication_uncertain"));
        }
        let mut receipt = req.receipt();
        receipt.merged = Some(false);
        return Ok(receipt);
    }
    if req.task_snapshot["qa_policy"] == "skip" {
        let coder = req
            .coder_receipt
            .as_ref()
            .ok_or_else(|| err("adapter_coder_missing"))?;
        if coder["passed"] != true
            || coder["role"] != "coder"
            || coder["action"] != "coder"
            || coder["head_sha"].as_str() != req.head_sha.as_deref()
            || coder["cleanup_complete"] != true
            || coder["run_id"] != req.run_id
            || coder["repository"] != req.repository
        {
            return Err(err("adapter_coder_invalid"));
        }
    } else {
        let qa = req
            .qa_receipt
            .as_ref()
            .ok_or_else(|| err("adapter_qa_missing"))?;
        if qa["passed"] != true
            || qa["role"] != "qa"
            || qa["reviewed_sha"].as_str() != req.head_sha.as_deref()
            || qa["cleanup_complete"] != true
            || qa["run_id"] != req.run_id
            || qa["repository"] != req.repository
            || qa["action"] != "qa"
        {
            return Err(err("adapter_qa_invalid"));
        }
    }
    if git(req, &["rev-parse", "HEAD"], None).await?.trim() != req.head_sha.as_deref().unwrap_or("")
        || !git(
            req,
            &["status", "--porcelain", "--untracked-files=all"],
            None,
        )
        .await?
        .is_empty()
    {
        return Err(err("adapter_workspace_changed"));
    }
    let branch = format!("workboard/{}", req.run_id);
    let workspace = req.workspace.to_str().unwrap();
    effect(req, "push", false)?;
    authenticated_git(
        req,
        args(&[
            "-C",
            workspace,
            "push",
            &format!("--force-with-lease=refs/heads/{branch}:"),
            "origin",
            &format!(
                "{}:refs/heads/{branch}",
                req.head_sha.as_deref().unwrap_or("")
            ),
        ]),
    )
    .await?;
    effect(req, "push", true)?;
    let p = match pull(req).await? {
        Some(p) => p,
        None => {
            effect(req, "pr", false)?;
            let created=api(req,&format!("repos/{}/pulls",req.repository),Some(&json!({"title":format!("Workboard run {}",req.run_id),"body":format!("Run {}\nExact implementation {}\n{}",req.run_id,req.head_sha.as_deref().unwrap(),if req.task_snapshot["qa_policy"]=="skip" {"QA explicitly skipped for this ticket. Coder validation and self-review completed for this commit."} else {"Independent QA passed for this commit."}),"head":branch,"base":base["default_branch"]}))).await?;
            validate_pull(req, &created, &base)?;
            effect(req, "pr", true)?;
            created
        }
    };
    validate_pull(req, &p, &base)?;
    if req.merge_policy == "manual" {
        let mut receipt = req.receipt();
        receipt.pull_request_url = p["html_url"].as_str().map(String::from);
        receipt.merged = Some(false);
        return Ok(receipt);
    }
    let target = api(
        req,
        &format!(
            "repos/{}/git/ref/heads/{}",
            req.repository,
            base["default_branch"].as_str().unwrap()
        ),
        None,
    )
    .await?;
    if target["object"]["sha"].as_str() != req.base_sha.as_deref() {
        return Err(err("adapter_base_changed"));
    }
    let number = p["number"].as_u64().unwrap();
    let mut request = json!({"sha":req.head_sha,"merge_method":"merge"});
    request["commit_title"] = json!(format!("Merge Workboard run {}", req.run_id));
    merge_gates(req, &p, &base).await?;
    effect(req, "merge", false)?;
    // PUT is selected explicitly; a successful exit alone is never a merge receipt.
    gh(
        req,
        args(&[
            "api",
            "--hostname",
            "github.com",
            "--method",
            "PUT",
            &format!("repos/{}/pulls/{number}/merge", req.repository),
            "--input",
            "-",
        ]),
        Some(&request),
    )
    .await?;
    let observed = observe_pull(req, &p, &base).await?;
    if observed.merged == Some(true) {
        effect(req, "merge", true)?;
    }
    Ok(observed)
}

pub(super) fn readable_source(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path).map_err(|_| err("adapter_workspace_invalid"))?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        if meta.permissions().mode() & 0o777 != 0o755 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .map_err(|_| err("adapter_workspace_invalid"))?;
        }
        for child in std::fs::read_dir(path).map_err(|_| err("adapter_workspace_invalid"))? {
            readable_source(&child.map_err(|_| err("adapter_workspace_invalid"))?.path())?;
        }
    } else if meta.is_file() {
        let mode = if meta.permissions().mode() & 0o111 != 0 {
            0o755
        } else {
            0o644
        };
        if meta.permissions().mode() & 0o777 != mode {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .map_err(|_| err("adapter_workspace_invalid"))?;
        }
    } else {
        return Err(err("adapter_workspace_invalid"));
    }
    Ok(())
}
