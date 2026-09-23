//! Private-network direct source reads.
//!
//! The only URL accepted by this server contains an unpredictable grant ID;
//! no filesystem path is parsed from HTTP. Grants are consumed before opening
//! a file and can never be replayed.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

use crate::source_catalog::{CatalogError, SourceCatalog};
use crate::source_protocol::{SourceReadGrant, SourceReadMode, SourceTransport};
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// In-memory one-use direct grants. The key is an opaque server-generated
/// grant ID; values never contain a network-supplied path.
#[derive(Debug, Clone, Default)]
pub struct DirectGrantStore {
	grants: Arc<Mutex<HashMap<String, SourceReadGrant>>>,
}

impl DirectGrantStore {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	pub fn insert(&self, grant: SourceReadGrant) -> Result<(), DirectGrantError> {
		if grant.transport != SourceTransport::Direct {
			return Err(DirectGrantError::TransportMismatch);
		}
		grant
			.validate()
			.map_err(|_| DirectGrantError::InvalidGrant)?;
		if grant.is_expired() {
			return Err(DirectGrantError::Expired);
		}
		let mut grants = self.grants.lock().expect("direct grant mutex poisoned");
		if grants.contains_key(&grant.grant_id) {
			return Err(DirectGrantError::Replay);
		}
		grants.insert(grant.grant_id.clone(), grant);
		Ok(())
	}

	/// Remove and return a grant exactly once. Expired grants are removed too,
	/// preventing a later retry from observing stale authorization.
	pub fn take(&self, grant_id: &str) -> Result<SourceReadGrant, DirectGrantError> {
		let grant = self
			.grants
			.lock()
			.expect("direct grant mutex poisoned")
			.remove(grant_id)
			.ok_or(DirectGrantError::NotFound)?;
		if grant.is_expired() {
			return Err(DirectGrantError::Expired);
		}
		Ok(grant)
	}

	pub fn discard(&self, grant_id: &str) {
		self.grants
			.lock()
			.expect("direct grant mutex poisoned")
			.remove(grant_id);
	}
	pub fn reap_expired(&self) {
		self.grants
			.lock()
			.expect("direct grant mutex poisoned")
			.retain(|_, grant| !grant.is_expired());
	}
}

#[derive(Debug, thiserror::Error)]
pub enum DirectGrantError {
	#[error("direct grant was not found")]
	NotFound,
	#[error("direct grant was already consumed")]
	Replay,
	#[error("direct grant has expired")]
	Expired,
	#[error("direct grant has invalid fields")]
	InvalidGrant,
	#[error("direct grant transport is not direct")]
	TransportMismatch,
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error(transparent)]
	Io(#[from] io::Error),
}

#[derive(Debug)]
pub struct DirectServerHandle {
	pub local_addr: SocketAddr,
	task: tokio::task::JoinHandle<()>,
}

impl DirectServerHandle {
	pub fn abort(self) {
		self.task.abort();
	}
}

/// Bind and start the private-network endpoint. The returned handle owns the
/// accept loop; aborting it is cancellation-safe and does not retain grants.
pub async fn start_direct_server(
	catalog: Arc<SourceCatalog>,
	grants: DirectGrantStore,
	listen: SocketAddr,
) -> Result<DirectServerHandle, io::Error> {
	let listener = TcpListener::bind(listen).await?;
	let local_addr = listener.local_addr()?;
	let limit = Arc::new(Semaphore::new(64));
	let task = tokio::spawn(async move {
		loop {
			let Ok((stream, _peer)) = listener.accept().await else {
				break;
			};
			let permit = match limit.clone().acquire_owned().await {
				Ok(permit) => permit,
				Err(_) => break,
			};
			let catalog = catalog.clone();
			let grants = grants.clone();
			tokio::spawn(async move {
				let _permit = permit;
				if let Err(error) = serve_connection(stream, catalog, grants).await {
					tracing::debug!(%error, "source direct request failed");
				}
			});
		}
	});
	Ok(DirectServerHandle { local_addr, task })
}

async fn serve_connection(
	mut stream: TcpStream,
	catalog: Arc<SourceCatalog>,
	grants: DirectGrantStore,
) -> Result<(), DirectGrantError> {
	let request = read_request(&mut stream).await?;
	let Some(grant_id) = request_grant_id(&request) else {
		write_response(&mut stream, 404, "Not Found", 0, None).await?;
		return Ok(());
	};
	let grant = match grants.take(grant_id) {
		Ok(grant) => grant,
		Err(DirectGrantError::Expired) => {
			write_response(&mut stream, 410, "Gone", 0, None).await?;
			return Ok(());
		},
		Err(DirectGrantError::NotFound | DirectGrantError::Replay) => {
			write_response(&mut stream, 404, "Not Found", 0, None).await?;
			return Ok(());
		},
		Err(error) => return Err(error),
	};
	if request.method != "GET" {
		write_response(&mut stream, 405, "Method Not Allowed", 0, None).await?;
		return Ok(());
	}
	let open_grant = grant.clone();
	let open_result =
		tokio::task::spawn_blocking(move || catalog.open_grant(&open_grant))
			.await
			.map_err(|error| {
				DirectGrantError::Io(io::Error::other(format!(
					"source digest task failed: {error}"
				)))
			})?;
	let (resolved, file) = match open_result {
		Ok(value) => value,
		Err(CatalogError::Expired) => {
			write_response(&mut stream, 410, "Gone", 0, None).await?;
			return Ok(());
		},
		Err(CatalogError::VersionMismatch | CatalogError::DigestMismatch) => {
			write_response(&mut stream, 409, "Conflict", 0, None).await?;
			return Ok(());
		},
		Err(CatalogError::InvalidRange) => {
			write_response(&mut stream, 416, "Range Not Satisfiable", 0, None).await?;
			return Ok(());
		},
		Err(error) => {
			write_response(&mut stream, 404, "Not Found", 0, None).await?;
			return Err(error.into());
		},
	};
	let mut file = tokio::fs::File::from_std(file);
	let length = grant.length;
	let content_range = (grant.mode == SourceReadMode::Range).then(|| {
		format!(
			"bytes {}-{}/{}",
			grant.offset,
			grant.offset.saturating_add(length).saturating_sub(1),
			resolved.item.size
		)
	});
	write_response(&mut stream, 200, "OK", length, content_range.as_deref()).await?;
	let mut remaining = length;
	let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
	while remaining > 0 {
		let requested = (remaining as usize).min(buffer.len());
		let read = file.read(&mut buffer[..requested]).await?;
		if read == 0 {
			return Err(DirectGrantError::Io(io::Error::new(
				io::ErrorKind::UnexpectedEof,
				"source file ended before the authorized length",
			)));
		}
		stream.write_all(&buffer[..read]).await?;
		remaining -= read as u64;
	}
	stream.shutdown().await?;
	Ok(())
}

#[derive(Debug)]
struct Request {
	method: String,
	path: String,
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, io::Error> {
	let mut bytes = Vec::with_capacity(1024);
	let mut buffer = [0_u8; 1024];
	loop {
		let read = timeout(Duration::from_secs(10), stream.read(&mut buffer))
			.await
			.map_err(|_| {
				io::Error::new(io::ErrorKind::TimedOut, "source request timed out")
			})??;
		if read == 0 {
			return Err(io::Error::new(
				io::ErrorKind::UnexpectedEof,
				"source request ended",
			));
		}
		bytes.extend_from_slice(&buffer[..read]);
		if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
			break;
		}
		if bytes.len() > MAX_REQUEST_BYTES {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"source request too large",
			));
		}
	}
	let text = std::str::from_utf8(&bytes).map_err(|_| {
		io::Error::new(io::ErrorKind::InvalidData, "source request is not UTF-8")
	})?;
	let line = text.split("\r\n").next().ok_or_else(|| {
		io::Error::new(io::ErrorKind::InvalidData, "source request line missing")
	})?;
	let mut fields = line.split_ascii_whitespace();
	let method = fields.next().unwrap_or_default().to_owned();
	let path = fields.next().unwrap_or_default().to_owned();
	let version = fields.next().unwrap_or_default();
	if fields.next().is_some()
		|| method.is_empty()
		|| path.is_empty()
		|| !matches!(version, "HTTP/1.0" | "HTTP/1.1")
	{
		return Err(io::Error::new(
			io::ErrorKind::InvalidData,
			"invalid source request line",
		));
	}
	Ok(Request { method, path })
}

fn request_grant_id(request: &Request) -> Option<&str> {
	if request.method != "GET" {
		return None;
	}
	let grant_id = request.path.strip_prefix("/v1/source/read/")?;
	if grant_id.is_empty()
		|| grant_id.contains('/')
		|| grant_id.contains('?')
		|| grant_id.contains('%')
		|| !grant_id.bytes().all(|byte| {
			byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
		}) {
		return None;
	}
	Some(grant_id)
}

async fn write_response(
	stream: &mut TcpStream,
	status: u16,
	reason: &str,
	length: u64,
	content_range: Option<&str>,
) -> Result<(), io::Error> {
	let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {length}\r\nContent-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\nConnection: close\r\n"
    );
	if let Some(content_range) = content_range {
		response.push_str("Content-Range: ");
		response.push_str(content_range);
		response.push_str("\r\n");
	}
	response.push_str("\r\n");
	stream.write_all(response.as_bytes()).await
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::source_protocol::SourceReadMode;

	fn grant(id: &str) -> SourceReadGrant {
		SourceReadGrant {
			grant_id: id.into(),
			root_id: "root".into(),
			worker_item_id: "item".into(),
			worker_content_version: "version".into(),
			expected_sha256: None,
			mode: SourceReadMode::Range,
			offset: 0,
			length: 1,
			transport: SourceTransport::Direct,
			expires_at: std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs() as i64
				+ 60,
			max_bytes: 1,
		}
	}

	#[test]
	fn direct_grants_are_one_use_and_opaque() {
		let store = DirectGrantStore::new();
		store.insert(grant("g-1")).unwrap();
		assert!(store.take("/tmp/secret").is_err());
		assert!(store.take("g-1").is_ok());
		assert!(matches!(store.take("g-1"), Err(DirectGrantError::NotFound)));
		let request = Request {
			method: "GET".into(),
			path: "/v1/source/read/g-1".into(),
		};
		assert_eq!(request_grant_id(&request), Some("g-1"));
		let request = Request {
			method: "GET".into(),
			path: "/v1/source/read/%2Ftmp".into(),
		};
		assert!(request_grant_id(&request).is_none());
	}

	#[tokio::test]
	async fn endpoint_serves_only_the_granted_slice_once() {
		use crate::source_catalog::{
			modified_at_ms, quick_fingerprint, CatalogObservation, SourceRootConfig,
			PRIVACY_CATALOG,
		};

		let dir = tempfile::tempdir().unwrap();
		let root_path = dir.path().join("root");
		std::fs::create_dir(&root_path).unwrap();
		let media_path = root_path.join("book.bin");
		std::fs::write(&media_path, b"abcdef").unwrap();
		let metadata = std::fs::metadata(&media_path).unwrap();
		let modified_at_ms = modified_at_ms(&metadata);
		let catalog = Arc::new(
			SourceCatalog::open(
				dir.path().join("state"),
				vec![SourceRootConfig {
					root_id: "root".into(),
					label: "Root".into(),
					kind: "library".into(),
					privacy_mode: PRIVACY_CATALOG.into(),
					path: root_path,
					transport: SourceTransport::Direct,
					direct_base_url: Some("http://127.0.0.1".into()),
				}],
			)
			.unwrap(),
		);
		let snapshot = catalog
			.commit_scan(
				"root",
				vec![CatalogObservation {
					absolute_path: media_path.clone(),
					relative_path: "book.bin".into(),
					size: metadata.len(),
					modified_at_ms,
					quick_fingerprint: quick_fingerprint(
						&media_path,
						metadata.len(),
						modified_at_ms,
					)
					.unwrap(),
					media_type: Some("application/octet-stream".into()),
					metadata: None,
				}],
			)
			.unwrap();
		let item = &snapshot.items[0];
		let store = DirectGrantStore::new();
		let mut read = grant("slice-once");
		read.worker_item_id = item.worker_item_id.clone();
		read.worker_content_version = item.worker_content_version.clone();
		read.offset = 1;
		read.length = 3;
		read.max_bytes = 3;
		store.insert(read).unwrap();

		let server = start_direct_server(catalog, store, "127.0.0.1:0".parse().unwrap())
			.await
			.unwrap();
		let url = format!("http://{}/v1/source/read/slice-once", server.local_addr);
		let response = reqwest::get(&url).await.unwrap();
		assert_eq!(response.status(), reqwest::StatusCode::OK);
		assert_eq!(response.bytes().await.unwrap().as_ref(), b"bcd");
		assert_eq!(
			reqwest::get(&url).await.unwrap().status(),
			reqwest::StatusCode::NOT_FOUND
		);
		server.abort();
	}
}
