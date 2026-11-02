//! Azure DevOps Server identity and collection boundaries. No cloud discovery.
use super::*;

pub(super) fn collection_path(path: &str) -> Result<String> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.is_empty() || parts.len() > 16 {
        return Err(invalid(
            "Enter the HTTPS collection URL, including its collection path.",
        ));
    }
    let mut encoded = Vec::new();
    for part in parts {
        let decoded = urls::decode(part)
            .ok_or_else(|| invalid("The collection URL contains an invalid encoded path."))?;
        if !urls::segment(&decoded)
            || decoded.trim().is_empty()
            || matches!(
                decoded.to_ascii_lowercase().as_str(),
                "_apis" | "_git" | "_workitems" | "_settings"
            )
        {
            return Err(invalid(
                "Enter the HTTPS collection URL, not a project, repository or API URL. Empty or unsafe path components are not allowed.",
            ));
        }
        encoded.push(urls::encode(&decoded));
    }
    Ok(encoded.join("/"))
}

pub(super) fn authenticated_user(value: &Json) -> Result<&str> {
    let user = value
        .get("authenticatedUser")
        .ok_or(Error::Forge(ForgeFailure::AzureIdentityUnverified))?;
    let id = user
        .str("id")
        .filter(|id| valid_identity_id(id))
        .ok_or(Error::Forge(ForgeFailure::AzureIdentityUnverified))?;
    if user.bool("isActive") != Some(true) || user.bool("isContainer") == Some(true) {
        return fail(ForgeFailure::AzureIdentityUnverified);
    }
    // Do not label an impersonated/different authorized identity as this user.
    if let Some(authorized) = value.get("authorizedUser")
        && !authorized
            .str("id")
            .is_some_and(|authorized| authorized.eq_ignore_ascii_case(id))
    {
        return fail(ForgeFailure::AzureIdentityUnverified);
    }
    user.str("customDisplayName")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| user.str("providerDisplayName"))
        .filter(|s| {
            !s.trim().is_empty()
                && s.len() <= 512
                && !s.chars().any(char::is_control)
                && !s.trim().eq_ignore_ascii_case("anonymous")
        })
        .ok_or(Error::Forge(ForgeFailure::AzureIdentityUnverified))
}

fn valid_identity_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && id.bytes().any(|b| b.is_ascii_hexdigit() && b != b'0')
}

pub(super) fn repository_from_remote(config: ForgeConfig, remote: &str) -> Option<ForgeRepository> {
    let (authority, collection) = config.server().strip_prefix("https://")?.split_once('/')?;
    let path = if let Some(rest) = remote.strip_prefix("https://") {
        let (remote_authority, path) = rest.split_once('/')?;
        if !remote_authority.eq_ignore_ascii_case(authority) {
            return None;
        }
        path
    } else {
        let host = authority.split(':').next()?;
        let rest = remote.strip_prefix("ssh://")?;
        let (ssh_authority, remote_path) = rest.split_once('/')?;
        let ssh_authority = match ssh_authority.split_once('@') {
            Some((user, target))
                if !user.is_empty()
                    && user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) =>
            {
                target
            }
            Some(_) => return None,
            None => ssh_authority,
        };
        let (ssh_host, port) = ssh_authority
            .split_once(':')
            .map_or((ssh_authority, None), |(h, p)| (h, Some(p)));
        if !ssh_host.eq_ignore_ascii_case(host)
            || port.is_some_and(|p| p.parse::<u16>().ok().filter(|p| *p > 0).is_none())
        {
            return None;
        }
        // An SSH alias or another virtual-directory mapping needs manual selection;
        // never guess a collection from only the host or final collection name.
        remote_path
    };
    let (project_path, repository) = path.split_once("/_git/")?;
    let (remote_collection, project) = project_path.rsplit_once('/')?;
    if collection_path(remote_collection).ok()? != collection {
        return None;
    }
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    let project = urls::decode(project)?;
    let repository = urls::decode(repository)?;
    if !urls::segment(&project) || !urls::segment(&repository) {
        return None;
    }
    ForgeRepository::new(config, &format!("{project}/{repository}")).ok()
}
