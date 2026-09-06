/**
 * Readium Web Publication Manifest (RWPM) access for the console reader.
 *
 * Everything here talks to the server's existing native EPUB surfaces, which
 * are mounted by `apps/server/src/routers/api/v2/epub.rs` behind the same
 * `auth_middleware` as page streaming:
 *
 * | Route                                    | Used for                            |
 * | ---------------------------------------- | ----------------------------------- |
 * | `GET /api/v2/epub/{id}/manifest.json`    | RWPM (metadata, readingOrder, toc)  |
 * | `GET /api/v2/epub/{id}/positions.json`   | Readium positions list              |
 * | `GET /api/v2/epub/{id}/resource/{*path}` | package resources (XHTML/CSS/media) |
 *
 * The manifest emits *absolute* hrefs built from the request `Host`
 * (`ReadiumManifestGenerator::resource_url`). We never fetch those directly:
 * behind the dev proxy that host is the Vite origin, so an absolute href can
 * point at a different origin than the one holding the session cookie. Every
 * href is reduced to its package-relative path and re-built as a same-origin
 * path instead, which keeps the cookie in play for the manifest, the
 * resources, and every subresource an XHTML section pulls in itself.
 */
import { redirectToLogin } from '@stump/ui/graphql/client';

const RESOURCE_MARKER = '/resource/';

export type RwpmLink = {
	href: string;
	type?: string | null;
	title?: string | null;
	rel?: string[] | string | null;
	children?: RwpmLink[] | null;
};

export type RwpmManifest = {
	metadata?: {
		title?: string | null;
		language?: string | null;
		readingProgression?: string | null;
		numberOfPages?: number | null;
	} | null;
	links?: RwpmLink[] | null;
	readingOrder?: RwpmLink[] | null;
	resources?: RwpmLink[] | null;
	toc?: RwpmLink[] | null;
};

export type RwpmPosition = {
	href: string;
	type?: string | null;
	title?: string | null;
	locations?: {
		position?: number | null;
		progression?: number | null;
		totalProgression?: number | null;
	} | null;
};

export type RwpmPositions = {
	total: number;
	positions: RwpmPosition[];
};

/** `GET /api/v2/epub/{id}/resource/{*path}` for a package-relative path. */
export function resourceUrl(mediaId: string, packagePath: string): string {
	const encoded = packagePath
		.split('/')
		.map((segment) => encodeURIComponent(segment))
		.join('/');
	return `/api/v2/epub/${encodeURIComponent(mediaId)}/resource/${encoded}`;
}

/** `GET /api/v2/media/{id}/page/{page}` — 1-based *visible* page. */
export function pageUrl(mediaId: string, page: number): string {
	return `/api/v2/media/${encodeURIComponent(mediaId)}/page/${page}`;
}

/**
 * The package-relative path of a resource href, fragment dropped. Accepts the
 * manifest's absolute URLs, a same-origin path, or an already-relative path,
 * so a locator written by any Stump client resolves to the same key.
 *
 * Mirrors `packagePathFromHref` in the React reader
 * (`packages/browser/src/components/readers/epub/readium/locator.ts`).
 */
export function packagePathFromHref(href: string): string {
	const withoutFragment = href.split('#')[0] ?? href;
	try {
		const url = new URL(withoutFragment, 'https://stump.invalid');
		const marker = url.pathname.indexOf(RESOURCE_MARKER);
		if (marker >= 0) {
			return decodeURIComponent(url.pathname.slice(marker + RESOURCE_MARKER.length));
		}
		return decodeURIComponent(url.pathname.replace(/^\//, ''));
	} catch {
		return withoutFragment.replace(/^\//, '');
	}
}

/** The fragment of an href, or `undefined` when it has none. */
export function fragmentFromHref(href: string): string | undefined {
	const hash = href.indexOf('#');
	if (hash < 0) return undefined;
	return href.slice(hash + 1) || undefined;
}

async function fetchJson<T>(url: string, signal?: AbortSignal): Promise<T> {
	const response = await fetch(url, { credentials: 'include', signal });
	if (response.status === 401) {
		redirectToLogin();
		throw new Error('Your session expired. Sign in again to keep reading.');
	}
	if (!response.ok) {
		throw new Error(`${url} responded ${response.status}`);
	}
	return (await response.json()) as T;
}

export type Publication = {
	manifest: RwpmManifest;
	positions: RwpmPosition[];
};

/**
 * Load the manifest and its linked positions list.
 *
 * The positions list is discovered by media type on a `links[]` entry the way
 * `@readium/shared` does it (not by `rel`), and is required: it carries the
 * per-resource `position` and `totalProgression` the server itself computes,
 * so this reader reports the same numbers a Komga, Kobo, or KOReader client
 * would for the same place in the book.
 */
export async function openPublication(
	mediaId: string,
	signal?: AbortSignal
): Promise<Publication> {
	const base = `/api/v2/epub/${encodeURIComponent(mediaId)}`;
	const manifest = await fetchJson<RwpmManifest>(`${base}/manifest.json`, signal);
	if (!manifest.readingOrder?.length) {
		throw new Error('This EPUB has no reading order in its Readium manifest.');
	}
	const positionsHref = manifest.links?.find(
		(link) => link.type === 'application/vnd.readium.position-list+json'
	)?.href;
	const positionsPath = positionsHref
		? new URL(positionsHref, 'https://stump.invalid').pathname
		: `${base}/positions.json`;
	const { positions } = await fetchJson<RwpmPositions>(positionsPath, signal);
	if (!positions.length) {
		throw new Error('The Readium positions list for this EPUB is empty.');
	}
	return { manifest, positions };
}
