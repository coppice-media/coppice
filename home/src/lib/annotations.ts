import type {
	AnnotationKind,
	ConsoleAnnotationFieldsFragment,
	DeviceKind
} from '$lib/graphql/generated/graphql';

export const KIND_LABELS: Record<AnnotationKind, string> = {
	HIGHLIGHT: 'Highlight',
	NOTE: 'Note',
	BOOKMARK: 'Bookmark'
};

export const KIND_OPTIONS = (Object.keys(KIND_LABELS) as AnnotationKind[]).map((value) => ({
	value,
	label: KIND_LABELS[value]
}));

/**
 * How a source is labelled in the hub. Only two lanes carry annotations
 * today: the native one (`WEB` — this console's reader and the GraphQL API)
 * and liseur-sync, whose CAS records report the kind of the device that
 * pushed them. Komga, OPDS and the Kavita profile carry position data but no
 * annotation resource, so they never appear as a source — their device kinds
 * are still labelled here because `source` is a `DeviceKind`.
 */
export const SOURCE_LABELS: Record<DeviceKind, string> = {
	WEB: 'Web',
	KOBO: 'Kobo · NickelStump',
	KOREADER: 'KOReader',
	LISEUR: 'Liseur',
	API: 'API',
	OPDS: 'OPDS',
	MIHON: 'Mihon',
	KOMELIA: 'Komelia'
};

export const SOURCE_OPTIONS: { value: DeviceKind; label: string }[] = (
	['WEB', 'KOBO', 'KOREADER', 'LISEUR'] as DeviceKind[]
).map((value) => ({ value, label: SOURCE_LABELS[value] }));

/** `since` choices, as whole days back from now. */
export const SINCE_OPTIONS = [
	{ value: '7', label: 'Last 7 days' },
	{ value: '30', label: 'Last 30 days' },
	{ value: '90', label: 'Last 90 days' },
	{ value: '365', label: 'Last year' }
] as const;

/** Sentinel for a select whose "no filter" entry cannot be an empty string. */
export const ANY_OPTION = '__any__';

/** The hub's filters, as the URL-friendly values the page keeps in its query. */
export type AnnotationFacets = {
	kind: AnnotationKind | null;
	source: DeviceKind | null;
	/** A book, by media id */
	book: string | null;
	/** Whole days back from now */
	sinceDays: string | null;
	search: string | null;
};

export const NO_FACETS: AnnotationFacets = {
	kind: null,
	source: null,
	book: null,
	sinceDays: null,
	search: null
};

export function annotationFilter(facets: AnnotationFacets): Record<string, unknown> {
	const days = facets.sinceDays ? Number(facets.sinceDays) : null;
	return {
		...(facets.kind ? { kind: [facets.kind] } : {}),
		...(facets.source ? { source: [facets.source] } : {}),
		...(facets.book ? { mediaId: facets.book } : {}),
		...(facets.search ? { query: facets.search } : {}),
		...(days && Number.isFinite(days)
			? { since: new Date(Date.now() - days * 86_400_000).toISOString() }
			: {})
	};
}

export function activeFacets(
	facets: AnnotationFacets,
	bookTitle: (mediaId: string) => string
): { key: keyof AnnotationFacets; label: string }[] {
	const chips: { key: keyof AnnotationFacets; label: string }[] = [];
	if (facets.kind) chips.push({ key: 'kind', label: KIND_LABELS[facets.kind] });
	if (facets.source) chips.push({ key: 'source', label: SOURCE_LABELS[facets.source] });
	if (facets.book) chips.push({ key: 'book', label: bookTitle(facets.book) });
	if (facets.sinceDays) {
		const option = SINCE_OPTIONS.find((candidate) => candidate.value === facets.sinceDays);
		chips.push({ key: 'sinceDays', label: option?.label ?? `Last ${facets.sinceDays} days` });
	}
	if (facets.search) chips.push({ key: 'search', label: `“${facets.search}”` });
	return chips;
}

/** One book with the annotations that belong to it, in arrival order. */
export type AnnotationGroup = {
	book: ConsoleAnnotationFieldsFragment['book'];
	items: ConsoleAnnotationFieldsFragment[];
};

/**
 * The server returns book-contiguous items (books newest-first, annotations
 * inside a book in the order they were made), so grouping is a single pass
 * that preserves both orders.
 */
export function groupByBook(items: ConsoleAnnotationFieldsFragment[]): AnnotationGroup[] {
	const groups: AnnotationGroup[] = [];
	for (const item of items) {
		const last = groups.at(-1);
		if (last && last.book.key === item.book.key) last.items.push(item);
		else groups.push({ book: item.book, items: [item] });
	}
	return groups;
}

/**
 * The reader's deep-link query for an annotation's anchor. The reader opens at
 * it once and only writes progress after the reader actually moves, so
 * following a link never rewrites the book's position.
 */
export function readerAnchor(annotation: ConsoleAnnotationFieldsFragment): string {
	const params = new URLSearchParams();
	if (annotation.href) params.set('href', annotation.href);
	if (annotation.fragment) params.set('fragment', annotation.fragment);
	if (typeof annotation.progression === 'number') {
		params.set('progression', String(annotation.progression));
	}
	if (typeof annotation.page === 'number') params.set('page', String(annotation.page));
	const query = params.toString();
	return query ? `?${query}` : '';
}

/** `Chapter Three · page 42 · 62%` — whatever the anchor actually carries. */
export function anchorLabel(annotation: ConsoleAnnotationFieldsFragment): string | null {
	const { progression } = annotation;
	const percent =
		typeof progression === 'number' && Number.isFinite(progression)
			? `${Math.round(Math.min(1, Math.max(0, progression)) * 100)}%`
			: null;
	const parts = [
		annotation.chapterTitle,
		typeof annotation.page === 'number' ? `page ${annotation.page}` : null,
		percent
	].filter((part): part is string => Boolean(part));
	return parts.length ? parts.join(' · ') : null;
}
