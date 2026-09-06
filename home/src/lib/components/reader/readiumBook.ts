/**
 * A foliate-js "book" backed by Stump's Readium surfaces.
 *
 * foliate-js renders anything that implements its book interface (`sections`
 * with `load()`, plus `toc`/`resolveHref`/…), so the EPUB never has to be
 * downloaded as an archive: each spine resource is streamed from
 * `GET /api/v2/epub/{id}/resource/{*path}` with the session cookie.
 *
 * Sections are handed to the renderer as `blob:` documents rather than as the
 * server URL directly, for two reasons:
 *
 * 1. Scripted content. foliate's renderer must keep `allow-scripts` on its
 *    iframe (WebKit bug 218086), and the iframe is same-origin with this app,
 *    so an EPUB's own `<script>` would run with our session. Parsing the
 *    resource here lets us drop scripts, inline handlers and `javascript:`
 *    URLs before the document is ever live. foliate's own EPUB loader takes
 *    the same blob route.
 * 2. Subresources. An injected `<base href>` pointing back at the resource
 *    route keeps relative CSS/image/font references streaming from the server
 *    (same-origin, cookie attached) instead of being rewritten one by one.
 */
import { annotationRange, positionFor, type ReaderAnchor, type ReaderLocator } from './locator';
import {
	fragmentFromHref,
	packagePathFromHref,
	resourceUrl,
	type Publication,
	type RwpmLink
} from './rwpm';

/** Prefix for the synthetic hrefs used to draw stored annotations. */
export const ANNOTATION_HREF = 'stump-annotation:';

/** Relative size weights are only used for progress; the unit is arbitrary. */
const SIZE_SCALE = 1_000_000;

export type ReaderTocItem = {
	label: string;
	href: string;
	subitems?: ReaderTocItem[];
};

export type ReadiumSection = {
	id: string;
	packagePath: string;
	mediaType: string;
	title?: string;
	size: number;
	position?: number;
	totalProgression: number;
	load: () => Promise<string>;
	unload: () => void;
	createDocument: () => Promise<Document>;
	resolveHref: (href: string) => string;
};

export type ReadiumBook = {
	sections: ReadiumSection[];
	dir: 'ltr' | 'rtl';
	toc: ReaderTocItem[];
	metadata: { title?: string; language?: string };
	/** Locators for `ANNOTATION_HREF` targets, filled in as annotations load. */
	annotationLocators: Map<string, ReaderLocator>;
	packagePaths: string[];
	resolveHref: (href: string) => ReaderAnchor;
	isExternal: (href: string) => boolean;
	splitTOCHref: (href: string) => [string, string | undefined];
	getTOCFragment: (doc: Document, id: string | undefined) => Node | null;
	destroy: () => void;
};

const parser = new DOMParser();

function parseResource(source: string, mediaType: string): Document {
	if (/xml|xhtml/.test(mediaType)) {
		const doc = parser.parseFromString(source, 'application/xhtml+xml');
		if (!doc.querySelector('parsererror')) return doc;
	}
	return parser.parseFromString(source, 'text/html');
}

/**
 * Remove everything that could execute inside the renderer's iframe. The
 * iframe is same-origin with the console, so this is the only barrier between
 * an EPUB's scripted content and the reader's session.
 */
function stripScripting(doc: Document): void {
	for (const element of doc.querySelectorAll('script, iframe, object, embed')) element.remove();
	for (const element of doc.querySelectorAll('*')) {
		for (const attribute of [...element.attributes]) {
			const name = attribute.name.toLowerCase();
			if (name.startsWith('on')) {
				element.removeAttribute(attribute.name);
				continue;
			}
			if (
				(name === 'href' || name === 'src' || name === 'xlink:href') &&
				/^\s*javascript:/i.test(attribute.value)
			) {
				element.removeAttribute(attribute.name);
			}
		}
	}
}

function tocItems(links: RwpmLink[] | null | undefined): ReaderTocItem[] {
	if (!links?.length) return [];
	return links.map((link) => {
		const path = packagePathFromHref(link.href);
		const fragment = fragmentFromHref(link.href);
		return {
			label: link.title ?? path,
			href: fragment ? `${path}#${fragment}` : path,
			subitems: tocItems(link.children)
		};
	});
}

/**
 * Adapt an opened publication into a foliate book.
 *
 * Section weights come from the server's positions list: consecutive
 * `totalProgression` values are the server's own cumulative byte fractions,
 * so foliate's whole-publication progress matches the percentage the server
 * would compute for the same resource.
 */
export function makeReadiumBook(mediaId: string, publication: Publication): ReadiumBook {
	const { manifest, positions } = publication;
	const order = manifest.readingOrder ?? [];
	const packagePaths = order.map((link) => packagePathFromHref(link.href));
	const blobs = new Map<string, string>();

	const totals = packagePaths.map(
		(path, index) => positionFor(positions, path)?.locations?.totalProgression ?? index / order.length
	);

	const loadDocument = async (packagePath: string, mediaType: string): Promise<Document> => {
		const url = resourceUrl(mediaId, packagePath);
		const response = await fetch(url, { credentials: 'include' });
		if (!response.ok) throw new Error(`${packagePath} responded ${response.status}`);
		const doc = parseResource(await response.text(), mediaType);
		stripScripting(doc);
		return doc;
	};

	const sections: ReadiumSection[] = order.map((link, index) => {
		const packagePath = packagePaths[index]!;
		const mediaType = link.type ?? 'application/xhtml+xml';
		const totalProgression = totals[index] ?? 0;
		const nextTotal = totals[index + 1] ?? 1;

		return {
			id: packagePath,
			packagePath,
			mediaType,
			title: link.title ?? undefined,
			size: Math.max(1, Math.round((nextTotal - totalProgression) * SIZE_SCALE)),
			position: positionFor(positions, packagePath)?.locations?.position ?? undefined,
			totalProgression,
			createDocument: () => loadDocument(packagePath, mediaType),
			load: async () => {
				const cached = blobs.get(packagePath);
				if (cached) return cached;
				const doc = await loadDocument(packagePath, mediaType);
				const base = doc.createElement('base');
				base.setAttribute('href', new URL(resourceUrl(mediaId, packagePath), location.origin).href);
				(doc.head ?? doc.documentElement).prepend(base);
				const isXml = doc.contentType === 'application/xhtml+xml';
				const serialized = isXml
					? new XMLSerializer().serializeToString(doc)
					: `<!DOCTYPE html>${doc.documentElement.outerHTML}`;
				const url = URL.createObjectURL(
					new Blob([serialized], { type: isXml ? 'application/xhtml+xml' : 'text/html' })
				);
				blobs.set(packagePath, url);
				return url;
			},
			unload: () => {
				const url = blobs.get(packagePath);
				if (!url) return;
				URL.revokeObjectURL(url);
				blobs.delete(packagePath);
			},
			// Section-relative links (`../Text/Section0002.xhtml#note`) become
			// package-relative ones so `resolveHref` below can look them up.
			resolveHref: (href) => {
				const resolved = new URL(href, `https://stump.invalid/${packagePath}`);
				const path = decodeURIComponent(resolved.pathname.replace(/^\//, ''));
				return resolved.hash ? `${path}${resolved.hash}` : path;
			}
		};
	});

	const annotationLocators = new Map<string, ReaderLocator>();

	return {
		sections,
		packagePaths,
		annotationLocators,
		dir: manifest.metadata?.readingProgression === 'rtl' ? 'rtl' : 'ltr',
		toc: tocItems(manifest.toc),
		metadata: {
			title: manifest.metadata?.title ?? undefined,
			language: manifest.metadata?.language ?? undefined
		},
		resolveHref: (href) => {
			if (href.startsWith(ANNOTATION_HREF)) {
				const locator = annotationLocators.get(href.slice(ANNOTATION_HREF.length));
				if (!locator) throw new Error(`Unknown annotation ${href}`);
				const index = packagePaths.indexOf(packagePathFromHref(locator.href));
				if (index < 0) throw new Error(`Annotation ${href} is not in this edition`);
				return { index, anchor: (doc) => annotationRange(doc, locator) };
			}
			const path = packagePathFromHref(href);
			const index = packagePaths.indexOf(path);
			if (index < 0) throw new Error(`${href} is not in this publication`);
			const fragment = fragmentFromHref(href);
			return { index, anchor: (doc) => (fragment ? doc.getElementById(fragment) : null) ?? 0 };
		},
		isExternal: (href) => {
			if (href.startsWith(ANNOTATION_HREF)) return false;
			if (!/^[a-z][a-z0-9+.-]*:/i.test(href)) return false;
			try {
				return new URL(href).origin !== location.origin;
			} catch {
				return true;
			}
		},
		splitTOCHref: (href) => [packagePathFromHref(href), fragmentFromHref(href)],
		getTOCFragment: (doc, id) => (id ? doc.getElementById(id) : doc.body?.firstChild) ?? null,
		destroy: () => {
			for (const url of blobs.values()) URL.revokeObjectURL(url);
			blobs.clear();
		}
	};
}
