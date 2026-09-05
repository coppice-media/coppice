//! A tiny canned HTTP/1.1 server for tests: routes are matched by request
//! path (query included) and every request line is recorded.

use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
};

use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::TcpListener,
};

#[derive(Debug, Clone)]
pub struct CannedResponse {
	pub status: u16,
	pub content_type: &'static str,
	pub body: Vec<u8>,
	pub headers: Vec<(String, String)>,
}

impl CannedResponse {
	pub fn ok(content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
		Self {
			status: 200,
			content_type,
			body: body.into(),
			headers: Vec::new(),
		}
	}

	pub fn json(body: impl Into<Vec<u8>>) -> Self {
		Self::ok("application/json", body)
	}

	pub fn html(body: impl Into<Vec<u8>>) -> Self {
		Self::ok("text/html; charset=utf-8", body)
	}

	pub fn status(status: u16) -> Self {
		Self {
			status,
			content_type: "text/plain",
			body: Vec::new(),
			headers: Vec::new(),
		}
	}

	pub fn with_header(mut self, name: &str, value: &str) -> Self {
		self.headers.push((name.to_string(), value.to_string()));
		self
	}
}

#[derive(Debug, Clone)]
pub struct RecordedRequest {
	pub method: String,
	pub path: String,
	pub headers: Vec<(String, String)>,
	pub body: Vec<u8>,
}

pub struct MockServer {
	base_url: String,
	routes: Arc<Mutex<HashMap<String, CannedResponse>>>,
	requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl MockServer {
	/// Bind an ephemeral loopback port and serve `routes` (keyed by exact
	/// path, e.g. `/manga?limit=1`). Unknown paths get a 404.
	pub async fn spawn(routes: Vec<(&str, CannedResponse)>) -> Self {
		let listener = TcpListener::bind("127.0.0.1:0")
			.await
			.expect("bind mock server");
		let address = listener.local_addr().expect("mock server address");
		let routes: Arc<Mutex<HashMap<String, CannedResponse>>> = Arc::new(Mutex::new(
			routes
				.into_iter()
				.map(|(path, response)| (path.to_string(), response))
				.collect(),
		));
		let requests = Arc::new(Mutex::new(Vec::new()));
		let server_routes = routes.clone();
		let server_requests = requests.clone();
		tokio::spawn(async move {
			loop {
				let Ok((mut stream, _)) = listener.accept().await else {
					break;
				};
				let routes = server_routes.clone();
				let requests = server_requests.clone();
				tokio::spawn(async move {
					let Some(request) = read_request(&mut stream).await else {
						return;
					};
					let response = routes
						.lock()
						.expect("routes poisoned")
						.get(&request.path)
						.cloned()
						.unwrap_or_else(|| CannedResponse::status(404));
					let is_head = request.method == "HEAD";
					requests.lock().expect("requests poisoned").push(request);
					let mut head = format!(
						"HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
						response.status,
						reason(response.status),
						response.content_type,
						response.body.len()
					);
					for (name, value) in &response.headers {
						head.push_str(&format!("{name}: {value}\r\n"));
					}
					head.push_str("\r\n");
					let _ = stream.write_all(head.as_bytes()).await;
					if !is_head {
						let _ = stream.write_all(&response.body).await;
					}
					let _ = stream.shutdown().await;
				});
			}
		});
		Self {
			base_url: format!("http://{address}"),
			routes,
			requests,
		}
	}

	pub fn base_url(&self) -> &str {
		&self.base_url
	}

	pub fn url(&self, path: &str) -> String {
		format!("{}{path}", self.base_url)
	}

	pub fn set_route(&self, path: &str, response: CannedResponse) {
		self.routes
			.lock()
			.expect("routes poisoned")
			.insert(path.to_string(), response);
	}

	pub fn requests(&self) -> Vec<RecordedRequest> {
		self.requests.lock().expect("requests poisoned").clone()
	}

	pub fn request_count(&self, path: &str) -> usize {
		self.requests()
			.iter()
			.filter(|request| request.path == path)
			.count()
	}
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<RecordedRequest> {
	let mut buffer = Vec::with_capacity(4096);
	let mut chunk = [0u8; 4096];
	let header_end = loop {
		let read = stream.read(&mut chunk).await.ok()?;
		if read == 0 {
			return None;
		}
		buffer.extend_from_slice(&chunk[..read]);
		if let Some(position) = find_header_end(&buffer) {
			break position;
		}
		if buffer.len() > 1 << 20 {
			return None;
		}
	};
	let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
	let mut lines = head.split("\r\n");
	let request_line = lines.next()?;
	let mut parts = request_line.split_whitespace();
	let method = parts.next()?.to_string();
	let path = parts.next()?.to_string();
	let headers: Vec<(String, String)> = lines
		.filter_map(|line| line.split_once(':'))
		.map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
		.collect();
	let content_length = headers
		.iter()
		.find(|(name, _)| name == "content-length")
		.and_then(|(_, value)| value.parse::<usize>().ok())
		.unwrap_or(0);
	let mut body = buffer[header_end + 4..].to_vec();
	while body.len() < content_length {
		let read = stream.read(&mut chunk).await.ok()?;
		if read == 0 {
			break;
		}
		body.extend_from_slice(&chunk[..read]);
	}
	Some(RecordedRequest {
		method,
		path,
		headers,
		body,
	})
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
	buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn reason(status: u16) -> &'static str {
	match status {
		200 => "OK",
		301 => "Moved Permanently",
		302 => "Found",
		400 => "Bad Request",
		403 => "Forbidden",
		404 => "Not Found",
		405 => "Method Not Allowed",
		429 => "Too Many Requests",
		500 => "Internal Server Error",
		502 => "Bad Gateway",
		503 => "Service Unavailable",
		_ => "Status",
	}
}
