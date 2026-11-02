//! Commit links are derived locally. No credentials, HTTP requests or Git writes.
use crate::*;

#[derive(Clone, Debug)]
pub struct CommitBrowserLink {
    pub remote: Vec<u8>,
    pub provider: ForgeProvider,
    pub url: String,
}
impl Repository {
    pub fn commit_browser_links(
        &self,
        commit: &ObjectId,
        remote: Option<&[u8]>,
        config: Option<ForgeConfig>,
        cancel: &CancellationToken,
    ) -> Result<Vec<CommitBrowserLink>> {
        cancel.check()?;
        let remotes = self.remotes(cancel)?;
        if remote.is_some_and(|name| !remotes.iter().any(|r| r.name == name)) {
            return Err(Error::InvalidInput(
                "Selected remote no longer exists. Refresh and choose again.".into(),
            ));
        }
        let mut links = Vec::new();
        for r in remotes
            .iter()
            .filter(|r| remote.is_none_or(|name| r.name == name))
        {
            for url in &r.urls {
                cancel.check()?;
                let repository = match &config {
                    Some(c) => ForgeRepository::from_remote(c.clone(), url),
                    None => ForgeRepository::from_known_remote(url),
                };
                if let Some(repository) = repository {
                    let url = repository.commit_url(commit);
                    if !links
                        .iter()
                        .any(|l: &CommitBrowserLink| l.remote == r.name && l.url == url)
                    {
                        links.push(CommitBrowserLink {
                            remote: r.name.clone(),
                            provider: repository.config().provider(),
                            url,
                        });
                    }
                }
            }
        }
        Ok(links)
    }
}
