use std::path::Path;

use tokio::fs;

use crate::error::CoreResult;

pub mod encryption;

pub fn chain_optional_iter<T>(
	required: impl IntoIterator<Item = T>,
	optional: impl IntoIterator<Item = Option<T>>,
) -> Vec<T> {
	required
		.into_iter()
		.map(Some)
		.chain(optional)
		.flatten()
		.collect()
}

/// Moves a file, falling back to copy + remove when `rename` cannot be used
/// because source and target sit on different filesystems.
pub async fn move_file(source: &Path, target: &Path) -> CoreResult<()> {
	if fs::rename(source, target).await.is_ok() {
		return Ok(());
	}
	fs::copy(source, target).await?;
	fs::remove_file(source).await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_chain_optional_iter() {
		let required = vec![1, 2, 3];
		let optional = vec![Some(4), None, Some(5)];

		let res = chain_optional_iter(required, optional);
		assert_eq!(res, vec![1, 2, 3, 4, 5]);
	}
}
