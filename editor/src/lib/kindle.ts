/**
 * The USB side of "send to Kindle": `POST /api/v2/media/{id}/kindle-file`
 * answers with the file a Kindle mounted over USB can open — `azw3` when the
 * server's `boko` converted it, otherwise the book's own format — instead of
 * mailing it through Amazon. A `400` means the format cannot be sideloaded
 * (an EPUB with no converter behind it), a `403` a missing `DOWNLOAD_FILE`.
 *
 * The route is same-origin like every other REST surface this app talks to
 * (`/api/v2/auth/login`, `/api/v2/media/{id}/page/{n}`), so the session cookie
 * travels with `credentials: 'include'` and no host is ever hardcoded.
 */
import { redirectToLogin } from '@stump/ui/graphql/client';

/** Extension implied by the media types the route answers with. */
const EXTENSION_BY_TYPE: Record<string, string> = {
	'application/vnd.amazon.ebook': 'azw3',
	'application/x-mobi8-ebook': 'azw3',
	'application/x-mobipocket-ebook': 'mobi',
	'application/epub+zip': 'epub',
	'application/pdf': 'pdf',
	'text/plain': 'txt'
};

/**
 * The name the server asked the browser to save the file under
 * (`Content-Disposition: attachment; filename="<book>.<format>"`). Falls back
 * to the book id plus the extension its `Content-Type` implies, because a
 * download with no name is one the operator cannot find again.
 */
function filenameOf(mediaId: string, disposition: string, contentType: string): string {
	const parameter = /filename\s*=\s*(?:"([^"]*)"|([^;]*))/i.exec(disposition);
	const name = (parameter?.[1] ?? parameter?.[2])?.trim();
	if (name) return name;
	return `${mediaId}.${EXTENSION_BY_TYPE[contentType] ?? 'azw3'}`;
}

/** The server's JSON API error message, or `fallback` when it sent none. */
async function errorMessage(response: Response, fallback: string): Promise<string> {
	try {
		const payload = (await response.json()) as { message?: string; error?: string };
		return payload.message ?? payload.error ?? fallback;
	} catch {
		// A body that is not JSON (or is empty) keeps the fallback.
		return fallback;
	}
}

/**
 * Fetch the Kindle file for `mediaId` and hand it to the browser's downloader.
 * Rejects with the server's own message on failure. `converted` mirrors
 * `X-Stump-Kindle-Converted`: false means the book went as-is because it was
 * already a format the device reads.
 */
export async function downloadKindleFile(
	mediaId: string
): Promise<{ filename: string; bytes: number; converted: boolean }> {
	const response = await fetch(`/api/v2/media/${encodeURIComponent(mediaId)}/kindle-file`, {
		method: 'POST',
		credentials: 'include'
	});
	if (response.status === 401) {
		redirectToLogin();
		throw new Error('Your session expired. Sign in again to download the file.');
	}
	if (!response.ok) {
		throw new Error(await errorMessage(response, 'Unable to build the Kindle file.'));
	}

	const converted = response.headers.get('x-stump-kindle-converted') === 'true';
	const blob = await response.blob();
	const contentType = (response.headers.get('content-type') ?? blob.type).split(';')[0].trim();
	const filename = filenameOf(
		mediaId,
		response.headers.get('content-disposition') ?? '',
		contentType
	);
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement('a');
	anchor.href = url;
	anchor.download = filename;
	document.body.append(anchor);
	anchor.click();
	anchor.remove();
	// The blob URL has to outlive the click for the download to start.
	setTimeout(() => URL.revokeObjectURL(url), 0);
	return { filename, bytes: blob.size, converted };
}
