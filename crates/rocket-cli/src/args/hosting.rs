use super::*;
use crate::hosting::{Action, Options, WriteMode};
use rocket_git::{
    ForgeConfig, ForgeFilter, ForgeKind, ForgeMergeMethod, ForgeProvider, ForgeReviewAction,
    ForgeSearch, ForgeState,
};

pub(super) fn parse(mut args: Vec<OsString>) -> Parsed<Options> {
    if args.is_empty() {
        return Err("hosting requires account, list, pull, create, review or merge");
    }
    let sub = text(&args.remove(0))?;
    let mut values = vec![
        "--provider",
        "--server",
        "--token-env",
        "--username",
        "--api-version",
        "--fixture-api-port",
    ];
    let mut flags = vec![];
    if sub != "account" {
        values.push("--repository");
    }
    match sub.as_str() {
        "account" | "pull" => (),
        "list" => values.extend([
            "--kind",
            "--state",
            "--pages",
            "--search",
            "--author",
            "--assignee",
            "--work-item-state",
        ]),
        "create" => values.extend([
            "--source",
            "--target",
            "--title",
            "--body",
            "--source-repository",
        ]),
        "review" => values.extend(["--action", "--body", "--comments"]),
        "merge" => values.push("--method"),
        _ => return Err("unknown hosting subcommand"),
    }
    if matches!(sub.as_str(), "create" | "review" | "merge") {
        flags.push("--preview");
        values.push("--expect-review");
    }
    let tail = Tail::parse(args, &flags, &values)?;
    let provider =
        ForgeProvider::from_key(&tail.required("--provider")?).ok_or("unknown hosting provider")?;
    let server = tail
        .text("--server")?
        .unwrap_or_else(|| provider.default_server().into());
    let mut config = ForgeConfig::new(provider, &server)
        .map_err(|_| "invalid HTTPS hosting server; see --help")?;
    if let Some(version) = tail.text("--api-version")? {
        config = config
            .with_azure_api_version(&version)
            .map_err(|_| "invalid Azure Server API version")?;
    }
    if let Some(port) = tail.values.get("--fixture-api-port") {
        config = config
            .with_fixture(number(port, 1, 65535)? as u16)
            .map_err(|_| "invalid fixture port")?;
    }
    let token_env = tail.required("--token-env")?;
    if token_env.len() > 128
        || !token_env.ends_with("_TOKEN")
        || !token_env.starts_with(|c: char| c.is_ascii_uppercase())
        || !token_env
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(
            "--token-env requires an uppercase environment variable name ending in _TOKEN; never pass the token itself",
        );
    }
    let repository = tail.text("--repository")?;
    if sub != "account" && repository.as_ref().is_none_or(String::is_empty) {
        return Err("--repository requires the hosting project path");
    }
    let action = match sub.as_str() {
        "account" => {
            tail.count(0)?;
            Action::Account
        }
        "list" => {
            tail.count(0)?;
            let kind = match tail.text("--kind")?.as_deref().unwrap_or("pulls") {
                "pulls" => ForgeKind::Pulls,
                "issues" => ForgeKind::Issues,
                _ => return Err("--kind must be pulls or issues"),
            };
            let state = match tail.text("--state")?.as_deref().unwrap_or("open") {
                "open" => ForgeState::Open,
                "closed" => ForgeState::Closed,
                "all" => ForgeState::All,
                _ => return Err("--state must be open, closed or all"),
            };
            let filter = ForgeFilter {
                kind,
                state,
                work_item_state: tail.text("--work-item-state")?.unwrap_or_default(),
                search: ForgeSearch {
                    text: tail.text("--search")?.unwrap_or_default(),
                    author: tail.text("--author")?.unwrap_or_default(),
                    assignee: tail.text("--assignee")?.unwrap_or_default(),
                },
                reviewers: false,
            };
            if !filter.search.is_empty() {
                filter
                    .search
                    .validate_for(provider)
                    .map_err(|_| "invalid or unsupported hosting search filter")?;
            }
            if !filter.work_item_state.is_empty()
                && (!provider.is_azure() || kind != ForgeKind::Issues)
            {
                return Err("--work-item-state is only for Azure issues");
            }
            let pages = tail
                .values
                .get("--pages")
                .map(|v| number(v, 1, 20))
                .transpose()?
                .unwrap_or(1) as u32;
            Action::List { filter, pages }
        }
        "pull" => {
            tail.count(1)?;
            Action::Pull(number(&tail.positional[0], 1, u32::MAX as u64)?)
        }
        "create" => {
            tail.count(0)?;
            Action::Create {
                source: tail.required("--source")?,
                target: tail.required("--target")?,
                title: tail.required("--title")?,
                body: tail.text("--body")?.unwrap_or_default(),
                source_repository: tail.text("--source-repository")?,
                mode: write_mode(&tail)?,
            }
        }
        "review" => {
            tail.count(1)?;
            let action = match tail.required("--action")?.as_str() {
                "comment" => ForgeReviewAction::Comment,
                "approve" => ForgeReviewAction::Approve,
                "request-changes" => ForgeReviewAction::RequestChanges,
                _ => return Err("--action must be comment, approve or request-changes"),
            };
            Action::Review {
                number: number(&tail.positional[0], 1, u32::MAX as u64)?,
                action,
                body: tail.text("--body")?.unwrap_or_default(),
                comments: tail.values.get("--comments").map(PathBuf::from),
                mode: write_mode(&tail)?,
            }
        }
        "merge" => {
            tail.count(1)?;
            let method = match tail.required("--method")?.as_str() {
                "merge" => ForgeMergeMethod::Merge,
                "squash" => ForgeMergeMethod::Squash,
                "rebase" => ForgeMergeMethod::Rebase,
                _ => return Err("--method must be merge, squash or rebase"),
            };
            Action::Merge {
                number: number(&tail.positional[0], 1, u32::MAX as u64)?,
                method,
                mode: write_mode(&tail)?,
            }
        }
        _ => unreachable!(),
    };
    Ok(Options {
        config,
        token_env,
        username: tail.text("--username")?.unwrap_or_default(),
        repository,
        action,
    })
}
fn write_mode(t: &Tail) -> Parsed<WriteMode> {
    match (t.flag("--preview"), t.text("--expect-review")?) {
        (true, None) => Ok(WriteMode::Preview),
        (false, Some(pin)) if pin.len() == 64 && pin.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(WriteMode::Submit(pin.to_ascii_lowercase()))
        }
        _ => Err("hosting writes require exactly one of --preview or --expect-review SHA256"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parsed(extra: &[&str]) -> Parsed<crate::args::Options> {
        let mut args = vec![
            "hosting",
            "--provider",
            "gitlab",
            "--token-env",
            "REVIEW_TOKEN",
        ];
        args.extend(extra);
        crate::args::parse(args.into_iter().map(Into::into).collect())
    }
    #[test]
    fn hosting_commands_require_explicit_scope_credentials_and_write_pins() {
        assert!(parsed(&["account"]).is_err()); // command comes first, flags after it
        let parse = |v: &[&str]| crate::args::parse(v.iter().map(OsString::from).collect());
        let common = [
            "--provider",
            "gitlab",
            "--token-env",
            "REVIEW_TOKEN",
            "--repository",
            "team/資料",
        ];
        for command in [
            vec!["hosting", "list"],
            vec!["hosting", "pull", "42"],
            vec![
                "hosting",
                "review",
                "42",
                "--action",
                "comment",
                "--body",
                "hello",
                "--preview",
            ],
            vec!["hosting", "merge", "42", "--method", "squash", "--preview"],
        ] {
            let mut args = command;
            args.extend(common);
            assert!(parse(&args).is_ok(), "{args:?}");
        }
        for tail in [
            vec!["--action", "comment", "--body", "hello"],
            vec!["--action", "comment", "--preview", "--expect-review", "bad"],
            vec!["--action", "unexpected", "--preview"],
            vec![
                "--action",
                "comment",
                "--preview",
                "--password",
                "not-accepted",
            ],
        ] {
            let mut args = vec!["hosting", "review", "42"];
            args.extend(common);
            args.extend(tail);
            assert!(parse(&args).is_err());
        }
        for name in [
            "PATH",
            "_TOKEN",
            "lower_TOKEN",
            "AUTH-TOKEN",
            "AUTH_TOKEN=secret",
        ] {
            assert!(
                parse(&[
                    "hosting",
                    "account",
                    "--provider",
                    "gitlab",
                    "--token-env",
                    name
                ])
                .is_err()
            );
        }
        let v = parse(&[
            "hosting",
            "review",
            "42",
            "--provider",
            "gitlab",
            "--token-env",
            "REVIEW_TOKEN",
            "--repository",
            "team/repo",
            "--action",
            "comment",
            "--body",
            "--json",
            "--preview",
        ])
        .unwrap();
        assert!(!v.json);
    }
}
