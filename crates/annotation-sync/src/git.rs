//! The git sink: the markdown renderer on a git working tree.
//!
//! Per user the sink owns `<root>/<user_id>/` as a git work tree. Each export
//! renders every book (identical bytes to the [`crate::markdown`] sink),
//! skips unchanged files, commits what changed with the configured author,
//! and pushes the configured branch to the configured remote.
//!
//! Push failures are retried once: on a rejected push the sink fetches the
//! remote branch and rebases the local commits onto it; if that rebase
//! conflicts, the rebase is aborted, the branch is left untouched, and an
//! [`AnnotationSyncError::GitConflict`] is surfaced to the job.
//!
//! Auth: the optional `token` setting is offered as an `x-access-token`
//! user/pass credential over fetch and push. Local path remotes need no
//! credentials. Transport support comes from the linked libgit2 (the crate
//! builds `git2` with `default-features = false`, so no vendored
//! openssl/libssh2 is compiled in).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use stump_api_types::settings::{SettingDefinition, SettingValues};

use crate::error::AnnotationSyncError;
use crate::markdown::{file_name, render_book};
use crate::model::ExportBatch;
use crate::sink::{string_setting, Sink, SinkDescriptor, SinkState};

pub const GIT_SINK_ID: &str = "git";

fn git_settings() -> Vec<SettingDefinition> {
	vec![
	SettingDefinition {
		key: "remote_url",
		label: "Remote URL",
		description: "Git remote to publish to (https or ssh URL, or a local path for testing).",
		kind: stump_api_types::settings::SettingKind::String,
		default: serde_json::Value::Null,
		required: true,
		secret: false,
		help_url: None,
	},
	SettingDefinition {
		key: "branch",
		label: "Branch",
		description: "Branch to commit to and push (default: main).",
		kind: stump_api_types::settings::SettingKind::String,
		default: serde_json::json!("main"),
		required: false,
		secret: false,
		help_url: None,
	},
	SettingDefinition {
		key: "token",
		label: "Access token",
		description: "Token used to authenticate fetch/push against the remote.",
		kind: stump_api_types::settings::SettingKind::String,
		default: serde_json::Value::Null,
		required: false,
		secret: true,
		help_url: None,
	},
	SettingDefinition {
		key: "author_name",
		label: "Commit author name",
		description: "Name used for export commits (default: Stump).",
		kind: stump_api_types::settings::SettingKind::String,
		default: serde_json::json!("Stump"),
		required: false,
		secret: false,
		help_url: None,
	},
	SettingDefinition {
		key: "author_email",
		label: "Commit author email",
		description: "Email used for export commits (default: annotations@stump.local).",
		kind: stump_api_types::settings::SettingKind::String,
		default: serde_json::json!("annotations@stump.local"),
		required: false,
		secret: false,
		help_url: None,
	},
	]
}

pub struct GitSink {
	root: PathBuf,
	values: SettingValues,
}

impl GitSink {
	pub fn new(root: &Path, values: SettingValues) -> Self {
		Self {
			root: root.to_path_buf(),
			values,
		}
	}

	fn branch(&self) -> String {
		string_setting(&self.values, "branch", "main")
	}

	fn author_name(&self) -> String {
		string_setting(&self.values, "author_name", "Stump")
	}

	fn author_email(&self) -> String {
		string_setting(&self.values, "author_email", "annotations@stump.local")
	}

	fn token(&self) -> Option<String> {
		self.values
			.get("token")
			.and_then(serde_json::Value::as_str)
			.filter(|token| !token.is_empty())
			.map(str::to_owned)
	}

	fn remote_url(&self) -> Option<String> {
		self.values
			.get("remote_url")
			.and_then(serde_json::Value::as_str)
			.filter(|url| !url.is_empty())
			.map(str::to_owned)
	}
}

struct PublishResult {
	committed: bool,
	pushed: bool,
	commit_id: Option<String>,
}

/// Blocking git work: render files, commit, push with one rebase retry.
fn publish(
	dir: &Path,
	batch: &ExportBatch,
	branch: &str,
	author_name: &str,
	author_email: &str,
	token: Option<&str>,
	remote_url: Option<&str>,
) -> Result<PublishResult, AnnotationSyncError> {
	let repo = open_or_init(dir, branch)?;

	// ---- write files (skip byte-identical) --------------------------------
	let mut changed = false;
	for book in &batch.books {
		let content = render_book(book);
		let path = dir.join(file_name(book));
		let unchanged = std::fs::read(&path)
			.map(|existing| existing == content.as_bytes())
			.unwrap_or(false);
		if !unchanged {
			std::fs::write(&path, content.as_bytes())?;
			changed = true;
		}
	}

	let Some(remote_url) = remote_url.filter(|url| !url.is_empty()) else {
		// No remote configured: commit locally and stop.
		let commit_id = if changed {
			Some(commit_all(&repo, author_name, author_email, batch)?)
		} else {
			None
		};
		return Ok(PublishResult {
			committed: changed,
			pushed: false,
			commit_id,
		});
	};

	if changed {
		commit_all(&repo, author_name, author_email, batch)?;
	}
	// With no new files there may still be unpushed commits from a previous
	// failed push, so pushing is unconditional.
	let remote_name = ensure_remote(&repo, remote_url)?;
	push_with_retry(&repo, &remote_name, branch, token)?;

	let commit_id = repo
		.head()
		.ok()
		.and_then(|head| head.peel_to_commit().ok())
		.map(|commit| commit.id().to_string());
	Ok(PublishResult {
		committed: changed,
		pushed: true,
		commit_id,
	})
}

fn open_or_init(dir: &Path, branch: &str) -> Result<git2::Repository, AnnotationSyncError> {
	match git2::Repository::open(dir) {
		Ok(repo) => Ok(repo),
		Err(_) => {
			let mut options = git2::RepositoryInitOptions::new();
			options.bare(false).initial_head(&format!("refs/heads/{branch}"));
			Ok(git2::Repository::init_opts(dir, &options)?)
		},
	}
}

fn ensure_remote(repo: &git2::Repository, url: &str) -> Result<String, AnnotationSyncError> {
	match repo.find_remote("origin") {
		Ok(remote) => {
			if remote.url()? != url {
				repo.remote_set_url("origin", url)?;
			}
		},
		Err(_) => {
			repo.remote("origin", url)?;
		},
	}
	Ok("origin".to_owned())
}

fn commit_all(
	repo: &git2::Repository,
	author_name: &str,
	author_email: &str,
	batch: &ExportBatch,
) -> Result<String, AnnotationSyncError> {
	let mut index = repo.index()?;
	index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
	index.write()?;
	let tree_id = index.write_tree()?;
	let tree = repo.find_tree(tree_id)?;
	let signature = git2::Signature::now(author_name, author_email)?;

	let parents: Vec<git2::Commit> = match repo.head() {
		Ok(head) => vec![head.peel_to_commit()?],
		Err(_) => Vec::new(),
	};
	let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

	let message = format!(
		"Export annotations ({} books)\n\nStump annotation sync for user {}.",
		batch.books.len(),
		batch.user_id
	);
	let oid = repo.commit(
		Some("HEAD"),
		&signature,
		&signature,
		&message,
		&tree,
		&parent_refs,
	)?;
	Ok(oid.to_string())
}

fn push_with_retry(
	repo: &git2::Repository,
	remote_name: &str,
	branch: &str,
	token: Option<&str>,
) -> Result<(), AnnotationSyncError> {
	let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
	match push(repo, remote_name, &refspec, token) {
		Ok(()) => Ok(()),
		Err(error) => {
			// Retry exactly once: fetch the remote branch and rebase the local
			// commits onto it. A conflict aborts the rebase and surfaces.
			tracing::info!(
				branch,
				%error,
				"push rejected; fetching remote branch and rebasing once"
			);
			rebase_onto_remote(repo, remote_name, branch, token)?;
			push(repo, remote_name, &refspec, token)
		},
	}
}

fn push(
	repo: &git2::Repository,
	remote_name: &str,
	refspec: &str,
	token: Option<&str>,
) -> Result<(), AnnotationSyncError> {
	let mut remote = repo.find_remote(remote_name)?;
	let mut options = git2::PushOptions::new();
	options.remote_callbacks(auth_callbacks(token));
	remote.push(&[refspec], Some(&mut options))?;
	Ok(())
}

fn auth_callbacks(token: Option<&str>) -> git2::RemoteCallbacks<'static> {
	let mut callbacks = git2::RemoteCallbacks::new();
	if let Some(token) = token {
		let token = token.to_owned();
		callbacks.credentials(move |_url, _username, allowed| {
			if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
				git2::Cred::userpass_plaintext("x-access-token", &token)
			} else {
				git2::Cred::default()
			}
		});
	}
	callbacks
}

/// Fetches the remote branch and rebases the local commits onto it. Any
/// conflict aborts the rebase and returns [`AnnotationSyncError::GitConflict`].
fn rebase_onto_remote(
	repo: &git2::Repository,
	remote_name: &str,
	branch: &str,
	token: Option<&str>,
) -> Result<(), AnnotationSyncError> {
	{
		let mut remote = repo.find_remote(remote_name)?;
		let mut fetch_options = git2::FetchOptions::new();
		fetch_options.remote_callbacks(auth_callbacks(token));
		let refspec = format!("refs/heads/{branch}");
		remote.fetch(&[refspec.as_str()], Some(&mut fetch_options), None)?;
	}

	let remote_branch = repo.find_branch(&format!("{remote_name}/{branch}"), git2::BranchType::Remote)?;
	let remote_commit = remote_branch.get().peel_to_commit()?;
	let remote_annotated = repo.find_annotated_commit(remote_commit.id())?;

	// Rebase the current (local) branch onto the fetched remote head: every
	// local commit not reachable from the upstream is replayed on top of it.
	let mut rebase_options = git2::RebaseOptions::new();
	let mut rebase = repo.rebase(
		None,
		Some(&remote_annotated),
		Some(&remote_annotated),
		Some(&mut rebase_options),
	)?;

	// libgit2 signals a textual conflict by failing the commit for the
	// conflicting operation. `next` yields `Option<Result<..>>`: `None` ends
	// the rebase, `Some(Err(..))` surfaces a step failure, and `commit` fails
	// when the applied patch left unresolved conflicts in the index.
	let committer = git2::Signature::now("Stump", "annotations@stump.local")?;
	let step_result = loop {
		match rebase.next() {
			Some(Ok(_operation)) => {
				if let Err(error) = rebase.commit(None, &committer, None) {
					break Err(error);
				}
			},
			Some(Err(error)) => break Err(error),
			None => break Ok(()),
		}
	};

	match step_result {
		Ok(()) => {
			let signature = git2::Signature::now("Stump", "annotations@stump.local")?;
			rebase.finish(Some(&signature))?;
			Ok(())
		},
		Err(error) => {
			let _ = rebase.abort();
			Err(AnnotationSyncError::GitConflict(error.message().to_string()))
		},
	}
}

#[async_trait]
impl Sink for GitSink {
	fn id(&self) -> &'static str {
		GIT_SINK_ID
	}

	fn descriptor(&self) -> SinkDescriptor {
		SinkDescriptor {
			id: GIT_SINK_ID,
			name: "Git repository",
			description: "Commits the markdown export to a git work tree and pushes it to a remote, rebasing once onto the remote branch when the push is rejected.",
			settings: git_settings(),
		}
	}

	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError> {
		let dir = self.root.join(&batch.user_id);
		std::fs::create_dir_all(&dir)?;

		// libgit2 calls are blocking; keep them off the async context.
		let result = {
			let dir = dir.clone();
			let batch = batch.clone();
			let branch = self.branch();
			let author_name = self.author_name();
			let author_email = self.author_email();
			let token = self.token();
			let remote_url = self.remote_url();
			tokio::task::spawn_blocking(move || {
				publish(
					&dir,
					&batch,
					&branch,
					&author_name,
					&author_email,
					token.as_deref(),
					remote_url.as_deref(),
				)
			})
			.await
			.map_err(|error| AnnotationSyncError::sink(format!("git task panicked: {error}")))??
		};

		tracing::debug!(
			user_id = %batch.user_id,
			committed = result.committed,
			pushed = result.pushed,
			"git sink export complete"
		);

		Ok(SinkState {
			liseur_seq: state.liseur_seq,
			data: serde_json::json!({ "last_commit": result.commit_id }),
		})
	}
}
