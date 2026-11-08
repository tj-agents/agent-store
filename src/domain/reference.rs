use std::fmt;

use thiserror::Error;

use super::slug::{self, Slug, token};

/// The tracker that owns a work item, such as `github` or `azure-devops`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Provider(Slug);

impl Provider {
    pub const GITHUB: &str = "github";
    pub const AZURE_DEVOPS: &str = "azure-devops";

    pub fn parse(text: &str) -> Result<Self, slug::ParseError> {
        Slug::parse(text).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn is_known(&self) -> bool {
        matches!(self.as_str(), Self::GITHUB | Self::AZURE_DEVOPS)
    }
}

/// Where a provider's ids are unique: `github.com/<owner>/<repo>` or `dev.azure.com/<org>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scope(String);

impl Scope {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A work item's id within its scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeId(String);

impl NativeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A repository's tracker scope, used to resolve a bare `461` or `#461`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryScope {
    provider: Provider,
    scope: Scope,
}

/// A tracker work item: `provider:scope:native_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkItemRef {
    provider: Provider,
    scope: Scope,
    native_id: NativeId,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid provider")]
    Provider(#[source] slug::ParseError),
    #[error("scope `{0}` must be non-empty text without whitespace")]
    Scope(String),
    #[error("id `{0}` must be non-empty text without whitespace")]
    NativeId(String),
    #[error(
        "{provider} scopes are `github.com/<owner>/<repo>` or `dev.azure.com/<org>`, not `{scope}`"
    )]
    ScopeShape { provider: String, scope: String },
    #[error("{provider} ids are positive integers, not `{id}`")]
    NotNumeric { provider: String, id: String },
    #[error("`{0}` is a bare id, which needs the repository's tracker scope")]
    NeedsRepository(String),
    #[error("`{0}` is not an id, `#id`, a GitHub or Azure DevOps URL, or `provider:scope:id`")]
    Unrecognised(String),
}

impl WorkItemRef {
    pub fn new(provider: &str, scope: &str, native_id: &str) -> Result<Self, ParseError> {
        let provider = Provider::parse(provider).map_err(ParseError::Provider)?;
        let scope = if provider.is_known() {
            scope.to_ascii_lowercase()
        } else {
            scope.to_owned()
        };
        let scope = token(&scope).map(Scope).ok_or(ParseError::Scope(scope))?;
        let mut native_id = token(native_id)
            .map(NativeId)
            .ok_or_else(|| ParseError::NativeId(native_id.to_owned()))?;
        if provider.is_known() {
            if !is_positive_integer(native_id.as_str()) {
                return Err(ParseError::NotNumeric {
                    provider: provider.as_str().to_owned(),
                    id: native_id.0,
                });
            }
            native_id = NativeId(native_id.0.trim_start_matches('0').to_owned());
            let segments = scope.as_str().split('/').filter(|s| !s.is_empty()).count();
            let shaped = match provider.as_str() {
                Provider::GITHUB => scope.as_str().starts_with("github.com/") && segments == 3,
                _ => scope.as_str().starts_with("dev.azure.com/") && segments == 2,
            };
            if !shaped {
                return Err(ParseError::ScopeShape {
                    provider: provider.as_str().to_owned(),
                    scope: scope.0,
                });
            }
        }
        Ok(Self {
            provider,
            scope,
            native_id,
        })
    }

    /// Parses `461`, `#461`, a GitHub issue or pull request URL, an Azure DevOps work item URL, or
    /// `provider:scope:id`. A bare id resolves against `repository`.
    pub fn parse(text: &str, repository: Option<&RepositoryScope>) -> Result<Self, ParseError> {
        let text = text.trim();
        let bare = text.strip_prefix('#').unwrap_or(text);
        if is_positive_integer(bare) {
            let repository =
                repository.ok_or_else(|| ParseError::NeedsRepository(text.to_owned()))?;
            return Self::new(
                repository.provider.as_str(),
                repository.scope.as_str(),
                bare,
            );
        }
        if let Some(reference) = parse_url(text) {
            return reference;
        }
        if !text.contains("://")
            && let Some((provider, rest)) = text.split_once(':')
            && let Some((scope, id)) = rest.rsplit_once(':')
        {
            return Self::new(provider, scope, id);
        }
        Err(ParseError::Unrecognised(text.to_owned()))
    }

    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn native_id(&self) -> &NativeId {
        &self.native_id
    }
}

impl fmt::Display for WorkItemRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.provider.as_str(),
            self.scope.as_str(),
            self.native_id.as_str()
        )
    }
}

impl RepositoryScope {
    /// The tracker scope of a GitHub or Azure DevOps git remote URL, HTTPS or SSH.
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let (host, path) = split_host_path(url)?;
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let (provider, scope) = match (host.as_str(), segments.as_slice()) {
            ("github.com", [owner, repo, ..]) => (
                Provider::GITHUB,
                format!("github.com/{owner}/{}", repo.trim_end_matches(".git")),
            ),
            ("dev.azure.com", [org, ..])
            | ("ssh.dev.azure.com" | "vs-ssh.visualstudio.com", ["v3", org, ..]) => {
                (Provider::AZURE_DEVOPS, format!("dev.azure.com/{org}"))
            }
            (host, _) if host.ends_with(".visualstudio.com") => {
                let org = host.trim_end_matches(".visualstudio.com");
                (Provider::AZURE_DEVOPS, format!("dev.azure.com/{org}"))
            }
            _ => return None,
        };
        let reference = WorkItemRef::new(provider, &scope, "1").ok()?;
        Some(Self {
            provider: reference.provider,
            scope: reference.scope,
        })
    }
}

fn parse_url(text: &str) -> Option<Result<WorkItemRef, ParseError>> {
    if !text.contains("://") {
        return None;
    }
    let (host, path) = split_host_path(text)?;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (host.as_str(), segments.as_slice()) {
        ("github.com", [owner, repo, "issues" | "pull", id, ..]) => Some(WorkItemRef::new(
            Provider::GITHUB,
            &format!("github.com/{owner}/{repo}"),
            id,
        )),
        ("dev.azure.com", [org, _project, "_workitems", "edit", id, ..]) => Some(WorkItemRef::new(
            Provider::AZURE_DEVOPS,
            &format!("dev.azure.com/{org}"),
            id,
        )),
        (host, [.., "_workitems", "edit", id]) if host.ends_with(".visualstudio.com") => {
            let org = host.trim_end_matches(".visualstudio.com");
            Some(WorkItemRef::new(
                Provider::AZURE_DEVOPS,
                &format!("dev.azure.com/{org}"),
                id,
            ))
        }
        _ => None,
    }
}

/// Splits `scheme://[user@]host/path` or scp-style `user@host:path` into a lowercase host and path.
fn split_host_path(url: &str) -> Option<(String, &str)> {
    let (authority, path) = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/').unwrap_or((rest, ""))
    } else {
        url.split_once(':')?
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    Some((host.to_ascii_lowercase(), path))
}

fn is_positive_integer(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) && text.bytes().any(|b| b != b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> RepositoryScope {
        RepositoryScope::from_remote_url("https://github.com/tj-agents/core.git").unwrap()
    }

    fn parsed(text: &str) -> String {
        WorkItemRef::parse(text, Some(&core())).unwrap().to_string()
    }

    #[test]
    fn bare_and_hash_ids_resolve_against_the_repository() {
        assert_eq!(parsed("461"), "github:github.com/tj-agents/core:461");
        assert_eq!(parsed("#461"), "github:github.com/tj-agents/core:461");
    }

    #[test]
    fn bare_id_without_a_repository_is_refused() {
        assert_eq!(
            WorkItemRef::parse("461", None),
            Err(ParseError::NeedsRepository("461".to_owned()))
        );
    }

    #[test]
    fn github_issue_and_pull_urls() {
        assert_eq!(
            parsed("https://github.com/TJ-Agents/Agent/issues/76"),
            "github:github.com/tj-agents/agent:76"
        );
        assert_eq!(
            parsed("https://github.com/tj-agents/cli/pull/28?x=1#c"),
            "github:github.com/tj-agents/cli:28"
        );
    }

    #[test]
    fn azure_devops_urls_scope_to_the_organisation() {
        assert_eq!(
            parsed("https://dev.azure.com/Contoso/Web/_workitems/edit/12345"),
            "azure-devops:dev.azure.com/contoso:12345"
        );
        assert_eq!(
            parsed("https://contoso.visualstudio.com/Web/_workitems/edit/7/"),
            "azure-devops:dev.azure.com/contoso:7"
        );
    }

    #[test]
    fn explicit_form_accepts_any_provider() {
        assert_eq!(
            parsed("github:github.com/tj-agents/core:473"),
            "github:github.com/tj-agents/core:473"
        );
        assert_eq!(
            parsed("jira:example.atlassian.net:PROJ-12"),
            "jira:example.atlassian.net:PROJ-12"
        );
    }

    #[test]
    fn known_providers_need_numeric_ids() {
        assert!(matches!(
            WorkItemRef::new("github", "github.com/a/b", "abc"),
            Err(ParseError::NotNumeric { .. })
        ));
        assert!(WorkItemRef::new("github", "github.com/a/b", "0").is_err());
    }

    #[test]
    fn known_ids_are_canonical_and_scopes_shaped() {
        assert_eq!(
            parsed("github:github.com/a/b:0461"),
            "github:github.com/a/b:461"
        );
        assert!(matches!(
            WorkItemRef::new("github", "a/b", "461"),
            Err(ParseError::ScopeShape { .. })
        ));
        assert!(WorkItemRef::new("azure-devops", "dev.azure.com/org/project", "1").is_err());
        assert_eq!(
            parsed("https://contoso.visualstudio.com/DefaultCollection/Web/_workitems/edit/7"),
            "azure-devops:dev.azure.com/contoso:7"
        );
    }

    #[test]
    fn rejects_unrecognised_text() {
        assert!(matches!(
            WorkItemRef::parse("https://example.com/x", Some(&core())),
            Err(ParseError::Unrecognised(_))
        ));
        assert!(WorkItemRef::parse("not a ref", None).is_err());
    }

    #[test]
    fn remote_urls_map_to_scopes() {
        let scope = |url| RepositoryScope::from_remote_url(url).map(|r| r.scope.0);
        assert_eq!(
            scope("git@github.com:tj-agents/cli.git").as_deref(),
            Some("github.com/tj-agents/cli")
        );
        assert_eq!(
            scope("https://contoso@dev.azure.com/contoso/Web/_git/site").as_deref(),
            Some("dev.azure.com/contoso")
        );
        assert_eq!(
            scope("git@ssh.dev.azure.com:v3/contoso/Web/site").as_deref(),
            Some("dev.azure.com/contoso")
        );
        assert_eq!(
            scope("https://contoso.visualstudio.com/Web/_git/site").as_deref(),
            Some("dev.azure.com/contoso")
        );
        assert_eq!(
            scope("contoso@vs-ssh.visualstudio.com:v3/contoso/Web/site").as_deref(),
            Some("dev.azure.com/contoso")
        );
        assert_eq!(scope("https://gitlab.com/a/b.git"), None);
    }
}
