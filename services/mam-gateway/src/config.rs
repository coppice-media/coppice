use crate::error::{GatewayError, GatewayResult};
use std::{
	fmt, fs,
	net::{IpAddr, SocketAddr},
	path::{Path, PathBuf},
	str::FromStr,
	sync::Arc,
	time::Duration,
};
use url::Url;

const DEFAULT_BIND: &str = "0.0.0.0:8088";
const DEFAULT_MAM_BASE_URL: &str = "https://t.myanonamouse.net";
const DEFAULT_MAM_SEARCH_PATH: &str = "/tor/js/loadSearchJSON.php";
const DEFAULT_QBIT_BASE_URL: &str = "http://127.0.0.1:8080";
const DEFAULT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_RESULT_TTL_SECS: u64 = 900;
const DEFAULT_MAX_TRACKER_BODY_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_TORRENT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_QBIT_BODY_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_GRAB_BODY_BYTES: usize = 64 * 1024;
const DEFAULT_HANDOFF_ROOT: &str = "/downloads";
const DEFAULT_MAX_HANDOFF_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_SECRET_BYTES: usize = 128 * 1024;

const MAM_ALLOWED_HOSTS: &[&str] = &[
	"myanonamouse.net",
	"www.myanonamouse.net",
	"t.myanonamouse.net",
];

/// A value whose `Debug` implementation is intentionally redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(Arc<str>);

impl Secret {
	pub fn new(value: impl Into<String>) -> GatewayResult<Self> {
		let value = value.into();
		if value.is_empty() || value.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
			return Err(GatewayError::Config);
		}
		Ok(Self(Arc::from(value)))
	}

	pub(crate) fn expose(&self) -> &str {
		&self.0
	}
}

impl fmt::Debug for Secret {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("<redacted>")
	}
}

#[derive(Clone)]
pub struct GatewayConfig {
	pub(crate) bind_addr: SocketAddr,
	pub(crate) gateway_token: Secret,
	pub(crate) mam_base_url: Url,
	pub(crate) mam_search_path: String,
	pub(crate) mam_cookie: Secret,
	pub(crate) qbit_base_url: Url,
	pub(crate) qbit_username: Option<Secret>,
	pub(crate) qbit_password: Option<Secret>,
	pub(crate) request_timeout: Duration,
	pub(crate) result_ttl: Duration,
	pub(crate) max_tracker_body_bytes: usize,
	pub(crate) max_torrent_bytes: usize,
	pub(crate) handoff_root: PathBuf,
	pub(crate) max_handoff_bytes: u64,
	pub(crate) max_qbit_body_bytes: usize,
	pub(crate) max_grab_body_bytes: usize,
}

impl GatewayConfig {
	/// Load configuration from the gateway process environment.
	///
	/// Secrets prefer the named file over the environment variable. No secret
	/// is included in a returned error or in the `Debug` output of this type.
	pub fn from_env() -> GatewayResult<Self> {
		let bind_addr = parse_value("MAM_GATEWAY_BIND", DEFAULT_BIND)?;
		let bind_addr = bind_addr.parse().map_err(|_| GatewayError::Config)?;
		let mam_base_url = parse_url(
			"MAM_BASE_URL",
			DEFAULT_MAM_BASE_URL,
			true,
			env_flag("MAM_ALLOW_INSECURE_FIXTURE"),
		)?;
		let qbit_base_url = parse_url("QBIT_URL", DEFAULT_QBIT_BASE_URL, false, true)?;
		let handoff_root =
			parse_path_root("MAM_GATEWAY_HANDOFF_ROOT", DEFAULT_HANDOFF_ROOT)?;
		let mam_search_path =
			parse_path("MAM_SEARCH_PATH", DEFAULT_MAM_SEARCH_PATH, false)?;
		let gateway_token =
			load_secret("MAM_GATEWAY_TOKEN", "MAM_GATEWAY_TOKEN_FILE", true)?
				.ok_or(GatewayError::Config)?;
		let mam_cookie = load_secret("MAM_COOKIE", "MAM_COOKIE_FILE", true)?
			.ok_or(GatewayError::Config)?;
		let qbit_username = load_secret("QBIT_USERNAME", "QBIT_USERNAME_FILE", false)?;
		let qbit_password = load_secret("QBIT_PASSWORD", "QBIT_PASSWORD_FILE", false)?;
		if qbit_username.is_some() != qbit_password.is_some() {
			return Err(GatewayError::Config);
		}

		Ok(Self {
			bind_addr,
			gateway_token,
			mam_base_url,
			mam_search_path,
			mam_cookie,
			qbit_base_url,
			qbit_username,
			qbit_password,
			request_timeout: parse_duration(
				"MAM_GATEWAY_TIMEOUT_SECS",
				DEFAULT_TIMEOUT_SECS,
				1,
				120,
			)?,
			result_ttl: parse_duration(
				"MAM_RESULT_TTL_SECS",
				DEFAULT_RESULT_TTL_SECS,
				60,
				86_400,
			)?,
			max_tracker_body_bytes: parse_size(
				"MAM_MAX_TRACKER_BODY_BYTES",
				DEFAULT_MAX_TRACKER_BODY_BYTES,
				1,
				16 * 1024 * 1024,
			)?,
			max_torrent_bytes: parse_size(
				"MAM_MAX_TORRENT_BYTES",
				DEFAULT_MAX_TORRENT_BYTES,
				1,
				32 * 1024 * 1024,
			)?,
			handoff_root,
			max_handoff_bytes: parse_u64(
				"MAM_GATEWAY_MAX_HANDOFF_BYTES",
				DEFAULT_MAX_HANDOFF_BYTES,
				1,
				64 * 1024 * 1024 * 1024,
			)?,
			max_qbit_body_bytes: parse_size(
				"QBIT_MAX_BODY_BYTES",
				DEFAULT_MAX_QBIT_BODY_BYTES,
				1,
				8 * 1024 * 1024,
			)?,
			max_grab_body_bytes: parse_size(
				"MAM_GATEWAY_MAX_GRAB_BODY_BYTES",
				DEFAULT_MAX_GRAB_BODY_BYTES,
				1,
				1024 * 1024,
			)?,
		})
	}

	/// Construct an operator configuration. This constructor accepts only the
	/// production MAM hosts and loopback qBittorrent endpoints.
	pub fn new(
		bind_addr: SocketAddr,
		mam_base_url: Url,
		qbit_base_url: Url,
		gateway_token: impl Into<String>,
		mam_cookie: impl Into<String>,
	) -> GatewayResult<Self> {
		Self::build(
			bind_addr,
			mam_base_url,
			qbit_base_url,
			gateway_token,
			mam_cookie,
			false,
		)
	}

	/// Test/fixture constructor. Fixture hosts are explicitly opt-in and are
	/// never accepted by `from_env` unless `MAM_ALLOW_INSECURE_FIXTURE=true`.
	pub fn for_tests(
		bind_addr: SocketAddr,
		mam_base_url: Url,
		qbit_base_url: Url,
		gateway_token: impl Into<String>,
		mam_cookie: impl Into<String>,
	) -> GatewayResult<Self> {
		Self::build(
			bind_addr,
			mam_base_url,
			qbit_base_url,
			gateway_token,
			mam_cookie,
			true,
		)
	}

	fn build(
		bind_addr: SocketAddr,
		mam_base_url: Url,
		qbit_base_url: Url,
		gateway_token: impl Into<String>,
		mam_cookie: impl Into<String>,
		allow_fixture_hosts: bool,
	) -> GatewayResult<Self> {
		validate_mam_base(&mam_base_url, allow_fixture_hosts)?;
		validate_qbit_base(&qbit_base_url)?;
		let gateway_token = Secret::new(gateway_token)?;
		let mam_cookie = Secret::new(mam_cookie)?;
		Ok(Self {
			bind_addr,
			gateway_token,
			mam_base_url,
			mam_search_path: DEFAULT_MAM_SEARCH_PATH.to_owned(),
			mam_cookie,
			qbit_base_url,
			qbit_username: None,
			qbit_password: None,
			request_timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
			result_ttl: Duration::from_secs(DEFAULT_RESULT_TTL_SECS),
			max_tracker_body_bytes: DEFAULT_MAX_TRACKER_BODY_BYTES,
			max_torrent_bytes: DEFAULT_MAX_TORRENT_BYTES,
			handoff_root: PathBuf::from(DEFAULT_HANDOFF_ROOT),
			max_handoff_bytes: DEFAULT_MAX_HANDOFF_BYTES,
			max_qbit_body_bytes: DEFAULT_MAX_QBIT_BODY_BYTES,
			max_grab_body_bytes: DEFAULT_MAX_GRAB_BODY_BYTES,
		})
	}

	pub fn bind_addr(&self) -> SocketAddr {
		self.bind_addr
	}

	pub fn max_grab_body_bytes(&self) -> usize {
		self.max_grab_body_bytes
	}

	pub fn with_handoff_root(mut self, path: impl Into<PathBuf>) -> GatewayResult<Self> {
		self.handoff_root = validate_handoff_root(path.into())?;
		Ok(self)
	}

	pub fn with_max_handoff_bytes(mut self, max_bytes: u64) -> GatewayResult<Self> {
		if max_bytes == 0 || max_bytes > 64 * 1024 * 1024 * 1024 {
			return Err(GatewayError::Config);
		}
		self.max_handoff_bytes = max_bytes;
		Ok(self)
	}
}

fn parse_value(name: &str, default: &str) -> GatewayResult<String> {
	Ok(std::env::var(name).unwrap_or_else(|_| default.to_owned()))
}

fn env_flag(name: &str) -> bool {
	matches!(
		std::env::var(name).ok().as_deref(),
		Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
	)
}

fn parse_url(
	name: &str,
	default: &str,
	mam: bool,
	allow_fixture_hosts: bool,
) -> GatewayResult<Url> {
	let value = parse_value(name, default)?;
	let url = Url::parse(&value).map_err(|_| GatewayError::Config)?;
	if mam {
		validate_mam_base(&url, allow_fixture_hosts)?;
	} else {
		validate_qbit_base(&url)?;
	}
	Ok(url)
}

fn parse_path_root(name: &str, default: &str) -> GatewayResult<PathBuf> {
	let value = parse_value(name, default)?;
	validate_handoff_root(PathBuf::from(value))
}

fn validate_handoff_root(path: PathBuf) -> GatewayResult<PathBuf> {
	if !path.is_absolute()
		|| path.to_string_lossy().len() > 4096
		|| path.components().any(|component| {
			matches!(
				component,
				std::path::Component::ParentDir | std::path::Component::CurDir
			)
		}) {
		return Err(GatewayError::Config);
	}
	Ok(path)
}

fn parse_path(name: &str, default: &str, allow_query: bool) -> GatewayResult<String> {
	let path = parse_value(name, default)?;
	if !path.starts_with('/')
		|| path.contains("..")
		|| path.bytes().any(|byte| byte == b'\r' || byte == b'\n')
		|| (!allow_query && (path.contains('?') || path.contains('#')))
	{
		return Err(GatewayError::Config);
	}
	Ok(path)
}

fn validate_mam_base(url: &Url, allow_fixture_hosts: bool) -> GatewayResult<()> {
	if url.scheme() != "https" && url.scheme() != "http" {
		return Err(GatewayError::Config);
	}
	if url.username() != ""
		|| url.password().is_some()
		|| url.fragment().is_some()
		|| url.query().is_some()
	{
		return Err(GatewayError::Config);
	}
	let host = url.host_str().ok_or(GatewayError::Config)?;
	let production_host = MAM_ALLOWED_HOSTS.iter().any(|allowed| host == *allowed);
	let fixture_host =
		allow_fixture_hosts && matches!(host, "127.0.0.1" | "localhost" | "::1");
	if !production_host && !fixture_host {
		return Err(GatewayError::Config);
	}
	if url.scheme() == "http" && !fixture_host {
		return Err(GatewayError::Config);
	}
	if production_host && !allow_fixture_hosts && url.port_or_known_default() != Some(443)
	{
		return Err(GatewayError::Config);
	}
	Ok(())
}

fn validate_qbit_base(url: &Url) -> GatewayResult<()> {
	if url.scheme() != "http"
		|| url.username() != ""
		|| url.password().is_some()
		|| url.query().is_some()
		|| url.fragment().is_some()
	{
		return Err(GatewayError::Config);
	}
	let host = url.host_str().ok_or(GatewayError::Config)?;
	let ip = IpAddr::from_str(host).map_err(|_| GatewayError::Config)?;
	if !ip.is_loopback() {
		return Err(GatewayError::Config);
	}
	Ok(())
}

fn load_secret(
	env_name: &str,
	file_name: &str,
	required: bool,
) -> GatewayResult<Option<Secret>> {
	let value = if let Ok(path) = std::env::var(file_name) {
		Some(read_secret_file(Path::new(&path))?)
	} else {
		std::env::var(env_name).ok()
	};
	match value {
		Some(value) => Ok(Some(Secret::new(value.trim().to_owned())?)),
		None if required => Err(GatewayError::Config),
		None => Ok(None),
	}
}

fn read_secret_file(path: &Path) -> GatewayResult<String> {
	let metadata = fs::metadata(path).map_err(|_| GatewayError::Config)?;
	if !metadata.is_file() || metadata.len() > MAX_SECRET_BYTES as u64 {
		return Err(GatewayError::Config);
	}
	fs::read_to_string(path).map_err(|_| GatewayError::Config)
}

fn parse_duration(
	name: &str,
	default: u64,
	min: u64,
	max: u64,
) -> GatewayResult<Duration> {
	let value = parse_value(name, &default.to_string())?
		.parse::<u64>()
		.map_err(|_| GatewayError::Config)?;
	if !(min..=max).contains(&value) {
		return Err(GatewayError::Config);
	}
	Ok(Duration::from_secs(value))
}

fn parse_size(
	name: &str,
	default: usize,
	min: usize,
	max: usize,
) -> GatewayResult<usize> {
	let value = parse_value(name, &default.to_string())?
		.parse::<usize>()
		.map_err(|_| GatewayError::Config)?;
	if !(min..=max).contains(&value) {
		return Err(GatewayError::Config);
	}
	Ok(value)
}

fn parse_u64(name: &str, default: u64, min: u64, max: u64) -> GatewayResult<u64> {
	let value = parse_value(name, &default.to_string())?
		.parse::<u64>()
		.map_err(|_| GatewayError::Config)?;
	if !(min..=max).contains(&value) {
		return Err(GatewayError::Config);
	}
	Ok(value)
}
