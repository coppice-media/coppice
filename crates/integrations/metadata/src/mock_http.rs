//! Test-only HTTP mock: a minimal blocking server that serves canned JSON
//! responses on `127.0.0.1:0` and records every request it receives.  Used to
//! point provider clients at a local endpoint without a network dependency.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One spawned mock server.  Serves exactly one canned response per
/// connection, in order, then stops accepting.
pub struct MockServer {
	pub url: String,
	requests: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
	/// Spin up a server that answers each incoming request with the next
	/// rendered response (see [`render_ok`]).
	pub fn spawn(responses: Vec<String>) -> Self {
		let listener =
			TcpListener::bind("127.0.0.1:0").expect("failed to bind mock server");
		let url = format!("http://{}", listener.local_addr().unwrap());
		let requests = Arc::new(Mutex::new(Vec::new()));
		let request_log = requests.clone();

		std::thread::spawn(move || {
			for response in responses {
				let Ok((mut stream, _)) = listener.accept() else {
					break;
				};
				let request = read_request(&mut stream);
				request_log
					.lock()
					.unwrap()
					.push(String::from_utf8_lossy(&request).to_string());
				let _ = stream.write_all(response.as_bytes());
				let _ = stream.flush();
			}
		});

		Self { url, requests }
	}

	/// Every request the server received so far, as raw HTTP/1.1 text.
	pub fn requests(&self) -> Vec<String> {
		self.requests.lock().unwrap().clone()
	}
}

/// Render a canned `200 OK` JSON response body.
pub fn render_ok(body: &str) -> String {
	format!(
		"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
		body.len(),
		body
	)
}

/// Read one full HTTP request (headers plus `Content-Length` body).
fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
	let mut buf = Vec::new();
	let mut chunk = [0u8; 1024];
	let header_end = loop {
		match stream.read(&mut chunk) {
			Ok(0) => return buf,
			Ok(n) => buf.extend_from_slice(&chunk[..n]),
			Err(_) => return buf,
		}
		if let Some(position) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
			break position + 4;
		}
	};

	let headers = String::from_utf8_lossy(&buf[..header_end]);
	let content_length = headers
		.lines()
		.find_map(|line| {
			let (name, value) = line.split_once(':')?;
			name.trim()
				.eq_ignore_ascii_case("content-length")
				.then(|| value.trim().parse::<usize>().ok())?
		})
		.unwrap_or(0);

	while buf.len() < header_end + content_length {
		match stream.read(&mut chunk) {
			Ok(0) => break,
			Ok(n) => buf.extend_from_slice(&chunk[..n]),
			Err(_) => break,
		}
	}
	buf
}
