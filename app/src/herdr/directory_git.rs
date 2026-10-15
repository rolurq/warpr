use std::path::PathBuf;

use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity as _};

use crate::code_review::git_repo_model::{GitRepoModels, GitRepoStatusEvent, GitRepoStatusModel};
use crate::code_review::github_repo_model::{GitHubRepoEvent, GitHubRepoModel};
use crate::context_chips::display_chip::GitLineChanges;
use crate::util::repo_detection::{RepoDetectionSessionType, detect_possible_git_repo};

/// What Warp knows about the git repository holding a directory, read from the same per-repo
/// models a terminal's prompt chips use.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectoryGitContext {
    pub repo_path: PathBuf,
    pub branch: String,
    /// `None` while the working tree is clean.
    pub diff_stats: Option<GitLineChanges>,
    /// The pull request GitHub reports for `branch`, as returned by `gh`; `None` while `gh` has
    /// not answered, is missing or unauthenticated, or the branch has no pull request.
    pub pull_request_url: Option<String>,
}

#[derive(Debug)]
pub enum HerdrDirectoryGitEvent {
    /// The context [`HerdrDirectoryGit::context`] returns may have changed.
    Changed,
}

struct RepoModels {
    repo_path: PathBuf,
    git_status: ModelHandle<GitRepoStatusModel>,
    github: ModelHandle<GitHubRepoModel>,
}

/// Follows the git repository of one directory: it resolves the repository containing the
/// directory and holds the shared per-repo status and GitHub models for as long as the directory
/// stays inside it. Nothing here talks to Herdr or to a Warp service.
pub struct HerdrDirectoryGit {
    directory: Option<PathBuf>,
    detecting: Option<SpawnedFutureHandle>,
    repo: Option<RepoModels>,
}

impl Entity for HerdrDirectoryGit {
    type Event = HerdrDirectoryGitEvent;
}

impl HerdrDirectoryGit {
    pub fn new() -> Self {
        Self {
            directory: None,
            detecting: None,
            repo: None,
        }
    }

    /// Follows `directory` instead of the previous one. Returns whether it differs.
    pub fn set_directory(
        &mut self,
        directory: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.directory == directory {
            return false;
        }
        if let Some(detecting) = self.detecting.take() {
            detecting.abort();
        }
        self.repo = None;
        self.directory = directory;
        let can_detect_repos = ctx.has_singleton_model::<DetectedRepositories>();
        if let Some(directory) = self.directory.as_ref().filter(|_| can_detect_repos) {
            let detection = detect_possible_git_repo(
                RepoDetectionSessionType::Local,
                &directory.to_string_lossy(),
                RepoDetectionSource::TerminalNavigation,
                ctx,
            );
            self.detecting = Some(ctx.spawn(detection, |me, repo, ctx| {
                me.detecting = None;
                if let Some(LocalOrRemotePath::Local(repo_path)) = repo {
                    me.follow_repo(repo_path, ctx);
                }
            }));
        }
        true
    }

    fn follow_repo(&mut self, repo_path: PathBuf, ctx: &mut ModelContext<Self>) {
        let location = LocalOrRemotePath::Local(repo_path.clone());
        let models = GitRepoModels::handle(ctx).update(ctx, |models, ctx| {
            let git_status = models.subscribe(&location, ctx)?;
            let github = models.subscribe_github_repo(&location, ctx)?;
            Ok::<_, anyhow::Error>((git_status, github))
        });
        let (git_status, github) = match models {
            Ok(models) => models,
            Err(error) => {
                log::warn!("Herdr directory git context unavailable: {error}");
                return;
            }
        };
        ctx.subscribe_to_model(
            &git_status,
            |me, source, GitRepoStatusEvent::MetadataChanged, ctx| {
                if me
                    .repo
                    .as_ref()
                    .is_some_and(|repo| repo.git_status == source)
                {
                    ctx.emit(HerdrDirectoryGitEvent::Changed);
                }
            },
        );
        ctx.subscribe_to_model(&github, |me, source, event, ctx| match event {
            GitHubRepoEvent::PrInfoChanged | GitHubRepoEvent::RepositoryInfoChanged => {
                if me.repo.as_ref().is_some_and(|repo| repo.github == source) {
                    ctx.emit(HerdrDirectoryGitEvent::Changed);
                }
            }
        });
        self.repo = Some(RepoModels {
            repo_path,
            git_status,
            github,
        });
        ctx.emit(HerdrDirectoryGitEvent::Changed);
    }

    /// The followed directory's repository, branch, diff stats and pull request; `None` when the
    /// directory is not inside a git repository or its status is not known yet.
    pub fn context(&self, app: &AppContext) -> Option<DirectoryGitContext> {
        let repo = self.repo.as_ref()?;
        let metadata = repo.git_status.as_ref(app).metadata(app)?;
        let branch = metadata.current_branch_name.trim();
        if branch.is_empty() {
            return None;
        }
        let diff_stats = Some(GitLineChanges::from_diff_stats(
            &metadata.stats_against_head,
        ))
        .filter(|changes| {
            changes.files_changed > 0 || changes.lines_added > 0 || changes.lines_removed > 0
        });
        Some(DirectoryGitContext {
            repo_path: repo.repo_path.clone(),
            branch: branch.to_owned(),
            diff_stats,
            pull_request_url: repo
                .github
                .as_ref(app)
                .pr_info(app)
                .map(|pr| pr.url.clone())
                .filter(|url| !url.trim().is_empty()),
        })
    }
}
