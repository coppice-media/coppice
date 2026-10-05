/**
 * Readium locator plumbing shared by the EPUB and paged readers.
 *
 * A locator written by this reader always uses the **package-relative** path
 * as `href` (`OEBPS/Text/Section0002.xhtml`), never the manifest's absolute
 * URL: the absolute form embeds the request host, and
 * `docs/developer/unified-reading-state` forbids turning a server-generated
 * URL into a publication resource href. Reads accept either form, because
 * `packagePathFromHref` normalizes both (the React reader and the Komga R2
 * route store absolute hrefs).
 */
import type { ReaderLocatorFieldsFragment, ReadiumLocatorInput } from '$lib/graphql/generated/graphql';

import { fragmentFromHref, packagePathFromHref, type RwpmPosition } from './rwpm';

export type ReaderLocator = ReaderLocatorFieldsFragment;

/** A place to open the renderer at: section index plus an in-section anchor. */
export type ReaderAnchor = {
	index: number;
	/** A fraction of the section, an `Element`, or a `Range`. */
	anchor: (doc: Document) => number | Element | Range | null;
};

/**
 * GraphQL `Decimal` arrives as a string (`"0.301"`), and codegen types it
 * `unknown` because the scalar has no TS mapping. Everything numeric that
 * crosses that boundary goes through here.
 */
export function decimal(value: unknown): number | undefined {
	if (value === null || value === undefined || value === '') return undefined;
	const parsed = Number(value);
	return Number.isFinite(parsed) ? parsed : undefined;
}

/** The position-list entry for a package path, if the server listed one. */
export function positionFor(
	positions: RwpmPosition[],
	packagePath: string
): RwpmPosition | undefined {
	return positions.find((position) => packagePathFromHref(position.href) === packagePath);
}

/**
 * Where to open the book: the stored locator's resource and in-resource
 * progression when it still exists in this edition, else the resource whose
 * `totalProgression` is closest to the stored percentage, else the start.
 */
export function resolveInitialAnchor(args: {
	packagePaths: string[];
	positions: RwpmPosition[];
	locator?: ReaderLocator | null;
	percentage?: number;
}): ReaderAnchor | null {
	const { packagePaths, positions, locator, percentage } = args;

	if (locator?.href) {
		const path = packagePathFromHref(locator.href);
		const index = packagePaths.indexOf(path);
		if (index >= 0) {
			const fragment = locator.locations?.fragments?.[0] ?? fragmentFromHref(locator.href);
			const progression = decimal(locator.locations?.progression) ?? 0;
			return {
				index,
				anchor: (doc) => (fragment ? doc.getElementById(fragment) : null) ?? progression
			};
		}
	}

	if (percentage !== undefined && percentage > 0) {
		let bestIndex = -1;
		let bestDelta = Number.POSITIVE_INFINITY;
		for (const [index, path] of packagePaths.entries()) {
			const total = positionFor(positions, path)?.locations?.totalProgression;
			if (total === null || total === undefined) continue;
			const delta = Math.abs(total - percentage);
			if (delta < bestDelta) {
				bestDelta = delta;
				bestIndex = index;
			}
		}
		if (bestIndex >= 0) return { index: bestIndex, anchor: () => 0 };
	}

	return null;
}

/**
 * The locator this reader persists for a relocation inside `packagePath`.
 *
 * `text` is deliberately absent: a position is not a selection, and the
 * converter contract forbids populating fields the source does not justify.
 */
export function locatorInputFor(args: {
	packagePath: string;
	mediaType: string;
	chapterTitle?: string | null;
	title?: string | null;
	progression: number;
	totalProgression: number;
	position?: number | null;
	fragment?: string | null;
}): ReadiumLocatorInput {
	const { packagePath, mediaType, chapterTitle, title, progression, totalProgression } = args;
	return {
		chapterTitle: chapterTitle ?? '',
		href: packagePath,
		title: title ?? undefined,
		type: mediaType,
		locations: {
			progression: Math.min(1, Math.max(0, progression)),
			totalProgression: Math.min(1, Math.max(0, totalProgression)),
			position: args.position ?? undefined,
			fragments: args.fragment ? [args.fragment] : undefined
		}
	};
}

/**
 * Build a book-relative locator from the selection's actual DOM range. The
 * text quote and its surrounding context are the durable anchor; progression
 * is measured from the range's text offset rather than the current page.
 */
export function locatorInputForSelection(args: {
	doc: Document;
	range: Range;
	packagePath: string;
	mediaType: string;
	chapterTitle?: string | null;
	title?: string | null;
	position?: number | null;
	totalProgressionStart: number;
	totalProgressionEnd: number;
}): { locator: ReadiumLocatorInput; excerpt: string } | null {
	const root = args.doc.body ?? args.doc.documentElement;
	const index = indexText(args.doc, root);
	const excerpt = args.range.toString().trim();
	if (!excerpt || !index.text.length) return null;

	const offsetAt = (node: Node, offset: number): number | null => {
		if (node.nodeType === Node.TEXT_NODE) {
			const entry = index.nodes.find((candidate) => candidate.node === node);
			return entry ? entry.start + Math.min(Math.max(offset, 0), entry.node.data.length) : null;
		}
		if (node !== root && !root.contains(node)) return null;
		try {
			const prefix = args.doc.createRange();
			prefix.setStart(root, 0);
			prefix.setEnd(node, offset);
			return prefix.toString().length;
		} catch {
			return null;
		}
	};

	const start = offsetAt(args.range.startContainer, args.range.startOffset);
	const end = offsetAt(args.range.endContainer, args.range.endOffset);
	if (start === null || end === null || end <= start) return null;

	const progression = Math.min(1, Math.max(0, start / index.text.length));
	const totalProgression =
		args.totalProgressionStart +
		progression * (args.totalProgressionEnd - args.totalProgressionStart);
	const before = index.text.slice(Math.max(0, start - SELECTION_CONTEXT_LENGTH), start).trim();
	const after = index.text.slice(end, end + SELECTION_CONTEXT_LENGTH).trim();
	const startElement =
		args.range.startContainer.nodeType === Node.ELEMENT_NODE
			? (args.range.startContainer as Element)
			: args.range.startContainer.parentElement;
	const fragment = startElement?.closest('[id]')?.id;

	return {
		excerpt,
		locator: {
			...locatorInputFor({
				packagePath: args.packagePath,
				mediaType: args.mediaType,
				chapterTitle: args.chapterTitle,
				title: args.title,
				position: args.position,
				progression,
				totalProgression,
				fragment
			}),
			text: { highlight: excerpt, before, after }
		}
	};
}

const SELECTION_CONTEXT_LENGTH = 80;

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

type TextIndex = {
	text: string;
	nodes: { node: Text; start: number }[];
};

function indexText(doc: Document, root: Node): TextIndex {
	const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);
	const nodes: { node: Text; start: number }[] = [];
	let text = '';
	while (walker.nextNode()) {
		const node = walker.currentNode as Text;
		nodes.push({ node, start: text.length });
		text += node.data;
	}
	return { text, nodes };
}

function rangeAt(doc: Document, index: TextIndex, start: number, end: number): Range | null {
	let startNode: { node: Text; start: number } | undefined;
	let endNode: { node: Text; start: number } | undefined;
	for (const entry of index.nodes) {
		if (entry.start <= start && start < entry.start + entry.node.data.length) startNode = entry;
		if (entry.start < end && end <= entry.start + entry.node.data.length) endNode = entry;
	}
	if (!startNode || !endNode) return null;
	const range = doc.createRange();
	range.setStart(startNode.node, start - startNode.start);
	range.setEnd(endNode.node, end - endNode.start);
	return range;
}

/**
 * Re-anchor a stored annotation inside a freshly loaded section document.
 *
 * Highlights are matched by their quoted text, whitespace-insensitively so a
 * different line-wrapping of the same XHTML still matches, and disambiguated
 * with the stored `before` and `after` context when the quote repeats. A CSS
 * selector or fragment id is used when there is no quote — that is all the
 * anchoring information a `ReadiumLocator` carries; anything else is invented.
 */
export function annotationRange(doc: Document, locator: ReaderLocator): Range | null {
	const selector = locator.locations?.cssSelector;
	const scope = (selector ? doc.querySelector(selector) : null) ?? doc.body;
	const quote = locator.text?.highlight?.trim();

	if (quote && scope) {
		const index = indexText(doc, scope);
		const pattern = new RegExp(quote.split(/\s+/).map(escapeRegExp).join('\\s+'), 'g');
		const before = locator.text?.before?.trim().split(/\s+/).join(' ');
		const after = locator.text?.after?.trim().split(/\s+/).join(' ');
		let fallback: Range | null = null;
		let exactMatch: Range | null = null;
		let exactDelta = Number.POSITIVE_INFINITY;
		const targetProgression = decimal(locator.locations?.progression);
		let contextualMatch: Range | null = null;
		let contextualMatches = 0;
		let occurrences = 0;
		let match: RegExpExecArray | null;
		while ((match = pattern.exec(index.text)) !== null) {
			const range = rangeAt(doc, index, match.index, match.index + match[0].length);
			if (!range) continue;
			occurrences += 1;
			fallback ??= range;
			const preceding = index.text.slice(0, match.index).split(/\s+/).join(' ').trimEnd();
			const following = index.text
				.slice(match.index + match[0].length)
				.split(/\s+/)
				.join(' ')
				.trimStart();
			const matchesBefore = !before || preceding.endsWith(before);
			const matchesAfter = !after || following.startsWith(after);
			if (matchesBefore && matchesAfter) {
				const delta =
					targetProgression === undefined
						? 0
						: Math.abs(match.index / index.text.length - targetProgression);
				if (delta < exactDelta) {
					exactMatch = range;
					exactDelta = delta;
				}
			}
			if (matchesBefore || matchesAfter) {
				contextualMatch ??= range;
				contextualMatches += 1;
			}
		}
		if (exactMatch) return exactMatch;
		if (contextualMatches === 1) return contextualMatch;
		if (before || after) return occurrences === 1 ? fallback : null;
		if (fallback) return fallback;
	}

	const anchorElement =
		(selector ? doc.querySelector(selector) : null) ??
		(locator.locations?.fragments?.[0]
			? doc.getElementById(locator.locations.fragments[0])
			: null) ??
		(fragmentFromHref(locator.href) ? doc.getElementById(fragmentFromHref(locator.href)!) : null);
	if (!anchorElement) return null;

	const range = doc.createRange();
	range.selectNodeContents(anchorElement);
	return range;
}
