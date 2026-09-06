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
 * with the stored `before` context when the quote repeats. A CSS selector or
 * a fragment id is used when there is no quote — that is all the anchoring
 * information a `ReadiumLocator` carries; anything else would be invented.
 */
export function annotationRange(doc: Document, locator: ReaderLocator): Range | null {
	const selector = locator.locations?.cssSelector;
	const scope = (selector ? doc.querySelector(selector) : null) ?? doc.body;
	const quote = locator.text?.highlight?.trim();

	if (quote && scope) {
		const index = indexText(doc, scope);
		const pattern = new RegExp(quote.split(/\s+/).map(escapeRegExp).join('\\s+'), 'g');
		const before = locator.text?.before?.trim().split(/\s+/).join(' ');
		let fallback: Range | null = null;
		let match: RegExpExecArray | null;
		while ((match = pattern.exec(index.text)) !== null) {
			const range = rangeAt(doc, index, match.index, match.index + match[0].length);
			if (!range) continue;
			if (!before) return range;
			const preceding = index.text.slice(0, match.index).split(/\s+/).join(' ');
			if (preceding.endsWith(before)) return range;
			fallback ??= range;
		}
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
