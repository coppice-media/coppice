//! Outbound source tunnel transport.
//!
//! A tunnel is a second authenticated WebSocket. It carries only binary
//! chunks for the already-issued grant; control JSON and media bytes never
//! share a socket.

use std::io;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::header, Message};

use crate::source_catalog::{CatalogError, SourceCatalog};
use crate::source_protocol::{SourceReadGrant, SourceTransport};

pub const TUNNEL_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SourceTunnelError {
	#[error("source tunnel grant must use tunnel transport")]
	TransportMismatch,
	#[error("source tunnel grant is invalid")]
	InvalidGrant,
	#[error("source tunnel URL is invalid: {0}")]
	InvalidUrl(String),
	#[error("source tunnel connection failed: {0}")]
	Connect(String),
	#[error(transparent)]
	Catalog(#[from] CatalogError),
	#[error("source digest task failed: {0}")]
	Task(String),
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error("source tunnel ended before the authorized byte count")]
	ShortRead,
}

/// Resolve and stream exactly one authorized grant over a separate outbound
/// WebSocket. `server` is the configured base URL, never a path supplied by a
/// control frame.
pub async fn handle_tunnel_grant(
	catalog: Arc<SourceCatalog>,
	server: &str,
	api_key: &str,
	grant: SourceReadGrant,
) -> Result<(), SourceTunnelError> {
	if grant.transport != SourceTransport::Tunnel {
		return Err(SourceTunnelError::TransportMismatch);
	}
	grant
		.validate()
		.map_err(|_| SourceTunnelError::InvalidGrant)?;
	let open_grant = grant.clone();
	let (_resolved, file) =
		tokio::task::spawn_blocking(move || catalog.open_grant(&open_grant))
			.await
			.map_err(|error| SourceTunnelError::Task(error.to_string()))??;
	let mut file = tokio::fs::File::from_std(file);
	let url = tunnel_url(server, &grant.grant_id);
	let mut request = url
		.into_client_request()
		.map_err(|error| SourceTunnelError::InvalidUrl(error.to_string()))?;
	request.headers_mut().insert(
		header::AUTHORIZATION,
		format!("Bearer {api_key}")
			.parse()
			.map_err(|_| SourceTunnelError::InvalidGrant)?,
	);
	let (socket, _) = tokio_tungstenite::connect_async(request)
		.await
		.map_err(|error| SourceTunnelError::Connect(error.to_string()))?;
	let (mut writer, _reader) = socket.split();
	let mut remaining = grant.length;
	let mut buffer = vec![0_u8; TUNNEL_CHUNK_BYTES];
	while remaining > 0 {
		let requested = (remaining as usize).min(buffer.len());
		let read = file.read(&mut buffer[..requested]).await?;
		if read == 0 {
			let _ = writer.close().await;
			return Err(SourceTunnelError::ShortRead);
		}
		writer
			.send(Message::Binary(buffer[..read].to_vec().into()))
			.await
			.map_err(|error| SourceTunnelError::Connect(error.to_string()))?;
		remaining -= read as u64;
	}
	// Sending close is the tunnel's terminal marker. The server hub also
	// verifies the byte count and rejects excess, short, or replayed transfers.
	writer
		.close()
		.await
		.map_err(|error| SourceTunnelError::Connect(error.to_string()))?;
	Ok(())
}

#[must_use]
pub fn tunnel_url(server: &str, grant_id: &str) -> String {
	let base = server.trim_end_matches('/');
	let base = if let Some(rest) = base.strip_prefix("https://") {
		format!("wss://{rest}")
	} else if let Some(rest) = base.strip_prefix("http://") {
		format!("ws://{rest}")
	} else {
		base.to_owned()
	};
	format!("{base}/api/v2/source-workers/streams/{grant_id}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tunnel_url_uses_source_route_and_ws_scheme() {
		assert_eq!(
			tunnel_url("https://example.test/", "grant-1"),
			"wss://example.test/api/v2/source-workers/streams/grant-1"
		);
		assert_eq!(
			tunnel_url("http://127.0.0.1:1", "grant-1"),
			"ws://127.0.0.1:1/api/v2/source-workers/streams/grant-1"
		);
	}

	#[tokio::test]
	async fn tunnel_opens_authenticated_socket_and_sends_exact_slice() {
		use crate::source_catalog::{
			modified_at_ms, quick_fingerprint, CatalogObservation, SourceRootConfig,
			PRIVACY_CATALOG,
		};
		use crate::source_protocol::SourceReadMode;
		use axum::{
			extract::{ws::WebSocketUpgrade, Path, State},
			http::HeaderMap,
			response::Response,
			routing::get,
			Router,
		};

		use sha2::Digest;
		async fn capture(
			State(tx): State<tokio::sync::mpsc::Sender<(String, String, Vec<u8>)>>,
			Path(grant_id): Path<String>,
			headers: HeaderMap,
			upgrade: WebSocketUpgrade,
		) -> Response {
			let authorization = headers
				.get("authorization")
				.and_then(|value| value.to_str().ok())
				.unwrap_or_default()
				.to_string();
			upgrade.on_upgrade(move |mut socket| async move {
				let mut bytes = Vec::new();
				while let Some(Ok(message)) = socket.recv().await {
					match message {
						axum::extract::ws::Message::Binary(chunk) => {
							bytes.extend_from_slice(&chunk)
						},
						axum::extract::ws::Message::Close(_) => break,
						_ => {},
					}
				}
				let _ = tx.send((grant_id, authorization, bytes)).await;
			})
		}

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
					transport: SourceTransport::Tunnel,
					direct_base_url: None,
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
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let (tx, mut rx) = tokio::sync::mpsc::channel(1);
		let app = Router::new()
			.route("/api/v2/source-workers/streams/{grant_id}", get(capture))
			.with_state(tx);
		let server = tokio::spawn(async move {
			axum::serve(listener, app).await.unwrap();
		});
		let grant = SourceReadGrant {
			grant_id: "grant-1".into(),
			root_id: "root".into(),
			worker_item_id: item.worker_item_id.clone(),
			worker_content_version: item.worker_content_version.clone(),
			expected_sha256: Some(format!("{:x}", sha2::Sha256::digest(b"abcdef"))),
			mode: SourceReadMode::Range,
			offset: 2,
			length: 3,
			transport: SourceTransport::Tunnel,
			expires_at: std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs() as i64
				+ 60,
			max_bytes: 3,
		};

		handle_tunnel_grant(
			catalog,
			&format!("http://{address}"),
			"source-secret",
			grant,
		)
		.await
		.unwrap();
		let (grant_id, authorization, bytes) =
			tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
				.await
				.unwrap()
				.unwrap();
		assert_eq!(grant_id, "grant-1");
		assert_eq!(authorization, "Bearer source-secret");
		assert_eq!(bytes, b"cde");
		server.abort();
	}
}
