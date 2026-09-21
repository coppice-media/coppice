//! The git sink: the markdown renderer on a git working tree.
//!
//! Per user the sink owns `<root>/<user_id>/` as a git work tree. Each export
//! renders every book (identical bytes to the [`crate::markdown`] sink),
//! skips unchanged files, commits what changed with the configured author,
//! and pushes the configured branch to the configured remote.
//!
//! A push rejected as non-fast-forward is retried once: the sink fetches the
//! remote branch and rebases the local commits onto it; if that rebase
//! conflicts, the rebase is aborted, the local branch is left untouched, and
//! an [`AnnotationSyncError::GitConflict`] is surfaced to the job.
//!
//! Auth: the optional `token` setting is offered as an `x-access-token`
//! user/pass credential over fetch and push. Local path remotes need no
//! credentials. Transport support comes from the linked libgit2 (the crate
//! builds `git2` with `default-features = false`, so no vendored
//! openssl/libssh2 is compiled in).

use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use stump_api_types::settings::{SettingDefinition, SettingKind, SettingValues};

use crate::error::AnnotationSyncError;
use crate::markdown::{
	acquire_user_lock, base_url_setting, preset_descriptors, user_dir_for_root,
	write_books_locked_for_git, RenderOptions,
};
use crate::model::ExportBatch;
use crate::sink::{string_setting, Sink, SinkDescriptor, SinkState};

pub const GIT_SINK_ID: &str = "git";
const REMOTE_NAME: &str = "origin";
fn git_settings() -> Vec<SettingDefinition> {
	vec![
		SettingDefinition {
			key: "remote_url",
			label: "Remote URL",
			description: "Remote to publish to (HTTPS or a local path outside this user's worktree). Leave empty to commit locally only.",
			kind: SettingKind::String,
			default: Value::Null,
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "branch",
			label: "Branch",
			description: "Branch to commit to and push.",
			kind: SettingKind::String,
			default: serde_json::json!("main"),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "token",
			label: "Access token",
			description: "Token used to authenticate fetch/push against the remote.",
			kind: SettingKind::String,
			default: Value::Null,
			required: false,
			secret: true,
			help_url: None,
		},
		SettingDefinition {
			key: "author_name",
			label: "Commit author name",
			description: "Name used for export commits.",
			kind: SettingKind::String,
			default: serde_json::json!("Coppice"),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "author_email",
			label: "Commit author email",
			description: "Email used for export commits.",
			kind: SettingKind::String,
			default: serde_json::json!("annotations@stump.local"),
			required: false,
			secret: false,
			help_url: None,
		},
		// Git and Markdown deliberately share the format-v2 layout settings.
		// Versionless rows remain on the old key/UUID path lane.
		SettingDefinition {
			key: "format_version",
			label: "Export format version",
			description: "Use version 2 for human filenames and structured frontmatter; omit for legacy UUID filenames.",
			kind: SettingKind::Int,
			default: serde_json::json!(2),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "preset",
			label: "Export preset",
			description: "A built-in safe path/body layout.",
			kind: SettingKind::Enum,
			default: serde_json::json!("obsidian"),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "destination",
			label: "Destination",
			description: "Contained relative directory below the per-user worktree.",
			kind: SettingKind::String,
			default: serde_json::json!(""),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "path_template",
			label: "Path template",
			description: "Safe relative template using {{author}}, {{title}}, {{source}}, {{source_id}}, or {{book_id}}.",
			kind: SettingKind::String,
			default: serde_json::json!("{{author}} - {{title}}/annotations.md"),
			required: false,
			secret: false,
			help_url: None,
		},
		SettingDefinition {
			key: "body_template",
			label: "Body template",
			description: "Optional constrained template using book fields and {{content}}.",
			kind: SettingKind::String,
			default: Value::Null,
			required: false,
			secret: false,
			help_url: None,
		},
		base_url_setting(),
	]
}

/// The sink's resolved settings.
#[derive(Debug, Clone)]
struct GitConfig {
	branch: String,
	author_name: String,
	author_email: String,
	token: Option<String>,
	remote_url: Option<String>,
	render: RenderOptions,
}

impl GitConfig {
	fn from_settings(values: &SettingValues) -> Self {
		let optional = |key: &str| {
			let value = string_setting(values, key, "");
			(!value.is_empty()).then_some(value)
		};
		Self {
			branch: string_setting(values, "branch", "main"),
			author_name: string_setting(values, "author_name", "Coppice"),
			author_email: string_setting(
				values,
				"author_email",
				"annotations@stump.local",
			),
			token: optional("token"),
			remote_url: optional("remote_url"),
			render: RenderOptions::from_settings(values),
		}
	}
}

pub struct GitSink {
	root: PathBuf,
	config: GitConfig,
}

impl GitSink {
	pub fn new(root: &Path, values: &SettingValues) -> Self {
		Self {
			root: root.to_path_buf(),
			config: GitConfig::from_settings(values),
		}
	}

	pub fn user_dir(&self, user_id: &str) -> PathBuf {
		self.root.join(user_id)
	}
}

struct PublishResult {
	committed: bool,
	pushed: bool,
	commit_id: Option<String>,
	path_map: BTreeMap<String, String>,
}

/// Blocking git work: render files, commit, push with one rebase retry.
fn publish(
	dir: &Path,
	batch: &ExportBatch,
	config: &GitConfig,
	state: &SinkState,
) -> Result<PublishResult, AnnotationSyncError> {
	let _lock = acquire_user_lock(dir)?;
	validate_branch(&config.branch)?;
	let repo = open_or_init(dir, &config.branch)?;
	let signature = git2::Signature::now(&config.author_name, &config.author_email)?;

	let write = write_books_locked_for_git(dir, batch, &config.render, Some(state))?;
	if write.written > 0 {
		commit_all(&repo, &signature, batch, &write.managed_paths)?;
	}

	// Push whenever the branch has commits: a previous run may have committed
	// and then failed to push, so an unchanged batch still publishes.
	let pushed = match &config.remote_url {
		Some(url) if repo.head().is_ok() => {
			ensure_remote(&repo, dir, url)?;
			push_with_retry(&repo, &config.branch, config.token.as_deref(), &signature)?;
			true
		},
		_ => false,
	};

	Ok(PublishResult {
		committed: write.written > 0,
		pushed,
		commit_id: head_commit_id(&repo),
		path_map: write.path_map,
	})
}

fn head_commit_id(repo: &git2::Repository) -> Option<String> {
	repo.head()
		.ok()
		.and_then(|head| head.peel_to_commit().ok())
		.map(|commit| commit.id().to_string())
}

fn open_or_init(
	dir: &Path,
	branch: &str,
) -> Result<git2::Repository, AnnotationSyncError> {
	std::fs::create_dir_all(dir)?;
	match git2::Repository::open(dir) {
		Ok(repo) => Ok(repo),
		Err(_) => {
			let mut options = git2::RepositoryInitOptions::new();
			options
				.bare(false)
				.initial_head(&format!("refs/heads/{branch}"));
			Ok(git2::Repository::init_opts(dir, &options)?)
		},
	}
}

fn ensure_remote(
	repo: &git2::Repository,
	worktree: &Path,
	url: &str,
) -> Result<(), AnnotationSyncError> {
	validate_remote_boundary(worktree, url)?;
	match repo.find_remote(REMOTE_NAME) {
		Ok(remote) => {
			if remote.url().unwrap_or_default() != url {
				repo.remote_set_url(REMOTE_NAME, url)?;
			}
		},
		Err(_) => {
			repo.remote(REMOTE_NAME, url)?;
		},
	}
	Ok(())
}

fn validate_branch(branch: &str) -> Result<(), AnnotationSyncError> {
	if branch.is_empty()
		|| branch.starts_with('/')
		|| branch.contains('\\')
		|| branch
			.split('/')
			.any(|part| part.is_empty() || part == "." || part == "..")
	{
		return Err(AnnotationSyncError::sink("invalid git branch"));
	}
	Ok(())
}

fn local_remote_path(url: &str) -> Option<PathBuf> {
	if let Some(path) = url.strip_prefix("file://") {
		return Some(PathBuf::from(path));
	}
	if url.contains("://") || url.starts_with("git@") {
		return None;
	}
	Some(PathBuf::from(url))
}

fn canonicalish(path: &Path) -> Result<PathBuf, AnnotationSyncError> {
	if path.exists() {
		return Ok(path.canonicalize()?);
	}
	let mut suffix = Vec::new();
	let mut current = path;
	while !current.exists() {
		if let Some(name) = current.file_name() {
			suffix.push(name.to_owned());
		}
		current = current.parent().ok_or_else(|| {
			AnnotationSyncError::sink("remote path has no existing parent")
		})?;
	}
	let mut output = current.canonicalize()?;
	for name in suffix.iter().rev() {
		output.push(name);
	}
	Ok(output)
}

fn validate_remote_boundary(
	worktree: &Path,
	url: &str,
) -> Result<(), AnnotationSyncError> {
	let Some(remote) = local_remote_path(url) else {
		return Ok(());
	};
	let worktree = canonicalish(worktree)?;
	let remote = canonicalish(&remote)?;
	if worktree == remote
		|| worktree.starts_with(&remote)
		|| remote.starts_with(&worktree)
	{
		return Err(AnnotationSyncError::sink(
			"git remote path must not overlap the export worktree",
		));
	}
	Ok(())
}

fn commit_all(
	repo: &git2::Repository,
	signature: &git2::Signature<'_>,
	batch: &ExportBatch,
	managed_paths: &[PathBuf],
) -> Result<String, AnnotationSyncError> {
	let mut index = repo.index()?;
	for path in managed_paths {
		index.add_path(path)?;
	}
	index.write()?;
	let tree_id = index.write_tree()?;
	let tree = repo.find_tree(tree_id)?;

	let parents: Vec<git2::Commit> = match repo.head() {
		Ok(head) => vec![head.peel_to_commit()?],
		Err(_) => Vec::new(),
	};
	let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

	let message = format!(
		"Export annotations ({} books)\n\nCoppice annotation sync for user {}.",
		batch.books.len(),
		batch.user_id
	);
	let oid = repo.commit(
		Some("HEAD"),
		signature,
		signature,
		&message,
		&tree,
		&parent_refs,
	)?;
	Ok(oid.to_string())
}

fn push_with_retry(
	repo: &git2::Repository,
	branch: &str,
	token: Option<&str>,
	signature: &git2::Signature<'_>,
) -> Result<(), AnnotationSyncError> {
	let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
	match push(repo, &refspec, token) {
		Ok(()) => Ok(()),
		Err(AnnotationSyncError::GitConflict(message)) => {
			// Retry exactly once: fetch the remote branch and rebase the local
			// commits onto it. A conflict aborts the rebase and surfaces.
			tracing::info!(
				branch,
				error = %message,
				"push rejected as non-fast-forward; rebasing onto the remote branch once"
			);
			rebase_onto_remote(repo, branch, token, signature)?;
			push(repo, &refspec, token)
		},
		Err(error) => Err(error),
	}
}

fn push(
	repo: &git2::Repository,
	refspec: &str,
	token: Option<&str>,
) -> Result<(), AnnotationSyncError> {
	let mut remote = repo.find_remote(REMOTE_NAME)?;
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
	branch: &str,
	token: Option<&str>,
	signature: &git2::Signature<'_>,
) -> Result<(), AnnotationSyncError> {
	{
		let mut remote = repo.find_remote(REMOTE_NAME)?;
		let mut fetch_options = git2::FetchOptions::new();
		fetch_options.remote_callbacks(auth_callbacks(token));
		let refspec = format!("refs/heads/{branch}");
		remote.fetch(&[refspec.as_str()], Some(&mut fetch_options), None)?;
	}

	let remote_branch =
		repo.find_branch(&format!("{REMOTE_NAME}/{branch}"), git2::BranchType::Remote)?;
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

	// `next` yields `None` once every operation is applied. A textual
	// conflict leaves the index unmerged, which makes `commit` fail; a patch
	// the remote already contains makes `commit` report `Applied` and is
	// simply skipped.
	let step_result = loop {
		match rebase.next() {
			Some(Ok(_operation)) => match rebase.commit(None, signature, None) {
				Ok(_) => {},
				Err(error) if error.code() == git2::ErrorCode::Applied => {},
				Err(error) => break Err(error),
			},
			Some(Err(error)) => break Err(error),
			None => break Ok(()),
		}
	};

	match step_result {
		Ok(()) => {
			rebase.finish(Some(signature))?;
			Ok(())
		},
		Err(error) => {
			let _ = rebase.abort();
			Err(AnnotationSyncError::GitConflict(
				error.message().to_string(),
			))
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
			description: "Commits safe markdown exports to a git work tree and pushes them to a remote, rebasing once onto the remote branch when the push is rejected.",
			settings: git_settings(),
			presets: preset_descriptors(),
		}
	}

	async fn export(
		&self,
		batch: &ExportBatch,
		state: &SinkState,
	) -> Result<SinkState, AnnotationSyncError> {
		// libgit2 calls are blocking; keep them off the async context.
		let dir = user_dir_for_root(&self.root, &batch.user_id)?;
		let config = self.config.clone();
		let is_v2 = config.render.format_version >= 2;
		let preset = config.render.preset.clone();
		let batch_for_task = batch.clone();
		let state_for_task = state.clone();
		let result = tokio::task::spawn_blocking(move || {
			publish(&dir, &batch_for_task, &config, &state_for_task)
		})
		.await
		.map_err(|error| {
			AnnotationSyncError::sink(format!("git task panicked: {error}"))
		})??;

		tracing::debug!(
			user_id = %batch.user_id,
			committed = result.committed,
			pushed = result.pushed,
			"git sink export complete"
		);

		let mut data = serde_json::Map::new();
		data.insert(
			"last_commit".to_owned(),
			serde_json::to_value(result.commit_id)?,
		);
		if is_v2 {
			data.insert("format_version".to_owned(), serde_json::json!(2));
			data.insert("preset".to_owned(), serde_json::json!(preset));
			data.insert("paths".to_owned(), serde_json::to_value(result.path_map)?);
		}
		Ok(SinkState {
			liseur_seq: state.liseur_seq,
			data: Value::Object(data),
		})
	}
}

#[cfg(test)]
mod tests {
	use git2::{Repository, RepositoryInitOptions, Signature};

	use super::*;
	use crate::markdown::{file_name, render_book};
	use crate::test_support::{at, fixture_batch, fixture_book};

	fn sink(root: &Path, remote: &Path) -> GitSink {
		let values: SettingValues = [
			(
				"remote_url".to_string(),
				Value::String(remote.to_string_lossy().into_owned()),
			),
			("branch".to_string(), Value::String("main".into())),
			("author_name".to_string(), Value::String("Tester".into())),
		]
		.into_iter()
		.collect();
		GitSink::new(root, &values)
	}

	fn bare_remote(path: &Path) -> Repository {
		let mut options = RepositoryInitOptions::new();
		options.bare(true).initial_head("refs/heads/main");
		Repository::init_opts(path, &options).unwrap()
	}

	fn remote_file(remote: &Repository, name: &str) -> Option<String> {
		let commit = remote
			.find_reference("refs/heads/main")
			.ok()?
			.peel_to_commit()
			.ok()?;
		let entry = commit.tree().ok()?.get_name(name)?.to_object(remote).ok()?;
		let blob = entry.as_blob()?;
		Some(String::from_utf8(blob.content().to_vec()).unwrap())
	}

	/// A second clone that writes to the remote behind the sink's back.
	fn other_clone(remote: &Path, dir: &Path) -> Repository {
		Repository::clone(&remote.to_string_lossy(), dir).unwrap()
	}

	fn commit_and_push(repo: &Repository, name: &str, content: &str) {
		let path = repo.workdir().unwrap().join(name);
		std::fs::write(path, content).unwrap();
		let mut index = repo.index().unwrap();
		index.add_path(Path::new(name)).unwrap();
		index.write().unwrap();
		let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
		let signature = Signature::now("Other", "other@example.com").unwrap();
		let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
		let parents: Vec<&git2::Commit> = parent.iter().collect();
		repo.commit(
			Some("HEAD"),
			&signature,
			&signature,
			&format!("other: {name}"),
			&tree,
			&parents,
		)
		.unwrap();
		repo.find_remote("origin")
			.unwrap()
			.push(&["refs/heads/main:refs/heads/main"], None)
			.unwrap();
	}

	#[tokio::test]
	async fn publishes_then_rebases_once_and_surfaces_conflicts() {
		let temp = tempfile::tempdir().unwrap();
		let remote_path = temp.path().join("remote.git");
		let remote = bare_remote(&remote_path);
		let root = temp.path().join("root");
		let sink = sink(&root, &remote_path);
		let render = RenderOptions::default();

		// First export: init, commit, push.
		let book = fixture_book();
		let batch = fixture_batch(vec![book.clone()]);
		let state = sink.export(&batch, &SinkState::default()).await.unwrap();
		let expected = render_book(&book, &render);
		assert_eq!(
			remote_file(&remote, &file_name(&book)).as_deref(),
			Some(expected.as_str())
		);
		let local = Repository::open(root.join("user-1")).unwrap();
		assert_eq!(
			state.data["last_commit"].as_str(),
			head_commit_id(&local).as_deref()
		);

		// Unchanged batch: no new commit, still a successful publish.
		let first_commit = head_commit_id(&local);
		sink.export(&batch, &state).await.unwrap();
		assert_eq!(head_commit_id(&local), first_commit);

		// Someone else pushes an unrelated file; our next push is rejected as
		// non-fast-forward, rebased once, and pushed.
		let other = other_clone(&remote_path, &temp.path().join("other"));
		commit_and_push(&other, "notes.md", "unrelated\n");

		let mut changed = book.clone();
		changed.annotations[0].note = Some("Updated note".to_string());
		let batch = fixture_batch(vec![changed.clone()]);
		let state = sink.export(&batch, &state).await.unwrap();

		assert_eq!(
			remote_file(&remote, "notes.md").as_deref(),
			Some("unrelated\n")
		);
		assert_eq!(
			remote_file(&remote, &file_name(&changed)).as_deref(),
			Some(render_book(&changed, &render).as_str())
		);
		assert_eq!(
			state.data["last_commit"].as_str(),
			remote
				.find_reference("refs/heads/main")
				.unwrap()
				.peel_to_commit()
				.unwrap()
				.id()
				.to_string()
				.as_str()
				.into(),
			"the rebased head is what the remote received"
		);
		assert_eq!(local.state(), git2::RepositoryState::Clean);

		// Someone else edits the same book file; the rebase conflicts, is
		// aborted, and the local branch keeps its commit.
		let other_head = other.find_remote("origin").unwrap();
		drop(other_head);
		{
			let mut origin = other.find_remote("origin").unwrap();
			origin.fetch(&["refs/heads/main"], None, None).unwrap();
			let fetched = other
				.find_reference("refs/remotes/origin/main")
				.unwrap()
				.peel_to_commit()
				.unwrap();
			other
				.reset(fetched.as_object(), git2::ResetType::Hard, None)
				.unwrap();
		}
		commit_and_push(&other, &file_name(&changed), "conflicting edit\n");

		let mut conflicting = changed.clone();
		conflicting.annotations[0].created_at = Some(at(99));
		let batch = fixture_batch(vec![conflicting.clone()]);
		let error = sink.export(&batch, &state).await.unwrap_err();
		assert!(
			matches!(error, AnnotationSyncError::GitConflict(_)),
			"expected a git conflict, got {error}"
		);
		assert_eq!(local.state(), git2::RepositoryState::Clean);
		let local_head = local.head().unwrap().peel_to_commit().unwrap();
		let local_content = local_head
			.tree()
			.unwrap()
			.get_name(&file_name(&conflicting))
			.unwrap()
			.to_object(&local)
			.unwrap();
		assert_eq!(
			local_content.as_blob().unwrap().content(),
			render_book(&conflicting, &render).as_bytes(),
			"the local commit survives the aborted rebase"
		);
		assert_eq!(
			remote_file(&remote, &file_name(&conflicting)).as_deref(),
			Some("conflicting edit\n"),
			"the remote is left untouched"
		);
	}

	#[tokio::test]
	async fn commits_locally_without_a_remote() {
		let temp = tempfile::tempdir().unwrap();
		let sink = GitSink::new(&temp.path().join("root"), &SettingValues::default());
		let batch = fixture_batch(vec![fixture_book()]);
		let state = sink.export(&batch, &SinkState::default()).await.unwrap();
		let repo = Repository::open(temp.path().join("root").join("user-1")).unwrap();
		assert_eq!(
			state.data["last_commit"].as_str(),
			head_commit_id(&repo).as_deref()
		);
		assert!(repo.find_remote(REMOTE_NAME).is_err());
	}
}
