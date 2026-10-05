/**
 * A foliate-js "book" backed by Stump's Readium surfaces.
 *
 * A publication adapter supplies its manifest, positions, and scoped
 * resource URLs; the native EPUB routes remain the default. Each spine section
 * is sanitized and rendered from a blob document so foliate's iframe never
 * executes publisher content with the app's origin or session.
 */
import DOMPurify from 'dompurify';
import { annotationRange, positionFor, type ReaderAnchor, type ReaderLocator } from './locator';
import {
	fragmentFromHref,
	packagePathFromHref,
	resourceUrl,
	type Publication,
	type ReaderResourceAdapter,
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

const FORBIDDEN_TAGS = [
	'script',
	'iframe',
	'frame',
	'frameset',
	'object',
	'applet',
	'embed',
	'fencedframe',
	'form',
	'input',
	'button',
	'select',
	'option',
	'textarea',
	'fieldset',
	'legend',
	'output',
	'datalist',
	'optgroup',
	'keygen'
];

/**
 * DOMPurify's HTML/SVG/MathML allowlist keeps normal EPUB markup and styles.
 * Publisher bases and refresh/CSP directives are removed before the
 * renderer adds its own base and scoped CSP to the blob document.
 *
 * In-place sanitizing needs an element root: passing the `Document` itself
 * throws. `<link>` is allowed so package stylesheets survive, then every
 * non-stylesheet link is dropped; `epub:type` is kept for footnote handling.
 */
function sanitizeDocument(doc: Document): void {
	for (const base of doc.querySelectorAll('base')) base.remove();
	for (const meta of doc.querySelectorAll('meta[http-equiv]')) {
		const directive = meta.getAttribute('http-equiv')?.trim().toLowerCase();
		if (directive === 'refresh' || directive === 'content-security-policy') meta.remove();
	}

	DOMPurify.sanitize(doc.documentElement, {
		IN_PLACE: true,
		WHOLE_DOCUMENT: true,
		ADD_TAGS: ['link'],
		ADD_ATTR: ['epub:type'],
		FORBID_TAGS: FORBIDDEN_TAGS,
		FORBID_ATTR: [
			'action',
			'autofocus',
			'form',
			'formaction',
			'formenctype',
			'formmethod',
			'formnovalidate',
			'formtarget',
			'ping',
			'srcdoc',
			'target'
		]
	});

	for (const link of doc.querySelectorAll('link')) {
		const rel = (link.getAttribute('rel') ?? '').toLowerCase().split(/\s+/).filter(Boolean);
		const stylesheet =
			rel.includes('stylesheet') && rel.every((token) => token === 'stylesheet' || token === 'alternate');
		if (!stylesheet) link.remove();
	}
}

function resourceSource(resourcePrefix: string): string {
	if (!resourcePrefix.trim()) return "'none'";
	try {
		const prefix = new URL(resourcePrefix, location.origin);
		if (
			prefix.origin !== location.origin ||
			prefix.username ||
			prefix.password ||
			prefix.search ||
			prefix.hash
		) {
			return "'none'";
		}
		const path = prefix.pathname.endsWith('/') ? prefix.pathname : `${prefix.pathname}/`;
		return `${prefix.origin}${path}`;
	} catch {
		return "'none'";
	}
}

function contentSecurityPolicy(resourcePrefix: string): string {
	const source = resourceSource(resourcePrefix);
	return [
		"default-src 'none'",
		"script-src 'none'",
		"connect-src 'none'",
		"form-action 'none'",
		"frame-src 'none'",
		"object-src 'none'",
		`base-uri ${source}`,
		`style-src ${source} 'unsafe-inline'`,
		`img-src ${source} data:`,
		`font-src ${source} data:`,
		`media-src ${source} data:`
	].join('; ');
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
export function makeReadiumBook(
	mediaId: string,
	publication: Publication,
	resourceAdapter?: ReaderResourceAdapter
): ReadiumBook {
	const { manifest, positions } = publication;
	const order = manifest.readingOrder ?? [];
	const packagePaths = order.map((link) => packagePathFromHref(link.href));
	const blobs = new Map<string, string>();
	const resourceForPath = (path: string) =>
		resourceAdapter?.resourceUrl(mediaId, path) ?? resourceUrl(mediaId, path);
	const resourcePrefix =
		resourceAdapter?.resourcePrefix ??
		(resourceAdapter ? resourceForPath('') : resourceUrl(mediaId, ''));
	const policy = contentSecurityPolicy(resourcePrefix);

	const totals = packagePaths.map(
		(path, index) => positionFor(positions, path)?.locations?.totalProgression ?? index / order.length
	);

	const loadDocument = async (packagePath: string, mediaType: string): Promise<Document> => {
		const url = resourceForPath(packagePath);
		const response = await fetch(url, { credentials: 'include' });
		if (!response.ok) throw new Error(`${packagePath} responded ${response.status}`);
		const doc = parseResource(await response.text(), mediaType);
		sanitizeDocument(doc);
		return doc;
	};

	const createSectionDocument = async (packagePath: string, mediaType: string): Promise<Document> => {
		const doc = await loadDocument(packagePath, mediaType);
		const namespace = 'http://www.w3.org/1999/xhtml';
		let head = doc.head;
		if (!head) {
			head = doc.createElementNS(namespace, 'head') as HTMLHeadElement;
			doc.documentElement.insertBefore(head, doc.documentElement.firstChild);
		}
		const base = doc.createElementNS(namespace, 'base');
		base.setAttribute('href', new URL(resourceForPath(packagePath), location.origin).href);
		const csp = doc.createElementNS(namespace, 'meta');
		csp.setAttribute('http-equiv', 'Content-Security-Policy');
		csp.setAttribute('content', policy);
		head.prepend(base);
		head.prepend(csp);
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
			createDocument: () => createSectionDocument(packagePath, mediaType),
			load: async () => {
				const cached = blobs.get(packagePath);
				if (cached) return cached;
				const doc = await createSectionDocument(packagePath, mediaType);
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
