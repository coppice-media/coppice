import type {
	AnnotationKind,
	ConsoleAnnotationFieldsFragment,
	DeviceKind,
} from '$lib/graphql/generated/graphql'

export type AnnotationSourceKind = DeviceKind | 'COPPICE'

export const KIND_LABELS: Record<AnnotationKind, string> = {
	HIGHLIGHT: 'Highlight',
	NOTE: 'Note',
	BOOKMARK: 'Bookmark',
}

export const KIND_OPTIONS = (Object.keys(KIND_LABELS) as AnnotationKind[]).map((value) => ({
	value,
	label: KIND_LABELS[value],
}))

/**
 * How a source is labelled in the hub. Only two lanes carry annotations
 * today: the native one (`WEB` — this console's reader and the GraphQL API)
 * and liseur-sync, whose CAS records report the kind of the device that
 * pushed them. Komga, OPDS and the Kavita and Audiobookshelf profiles carry
 * position data but no annotation resource, so they never appear as a source
 * — their device kinds are still labelled here because `source` is a
 * `DeviceKind`.
 */
export const SOURCE_LABELS: Record<AnnotationSourceKind, string> = {
	WEB: 'Web',
	KOBO: 'Kobo · NickelCoppice',
	KOREADER: 'KOReader',
	CROSSPOINT: 'CrossPoint',
	COPPICE: 'Coppice',
	LISEUR: 'Liseur',
	API: 'API',
	OPDS: 'OPDS',
	ABS: 'Audiobookshelf',
	MIHON: 'Mihon',
	KOMELIA: 'Komelia',
	KAVITA: 'Kavita',
	WORKER: 'Worker',
}

/** The sources that can actually carry annotations, for the source selector. */
export const SOURCE_OPTIONS: { value: AnnotationSourceKind; label: string }[] = (
	['WEB', 'KOBO', 'KOREADER', 'COPPICE', 'CROSSPOINT', 'LISEUR'] as AnnotationSourceKind[]
).map((value) => ({ value, label: SOURCE_LABELS[value] }))

/**
 * The two lanes annotations arrive through: native rows (`WEB`) and the
 * liseur-sync records a device pushed. A lane is the coarse cut the hub's
 * chips make; the source selector narrows a lane to one device kind.
 */
export type AnnotationLane = 'native' | 'device'

export const DEVICE_SOURCES: AnnotationSourceKind[] = ['KOBO', 'KOREADER', 'COPPICE', 'LISEUR']

export const LANE_OPTIONS: { value: AnnotationLane | 'all'; label: string }[] = [
	{ value: 'all', label: 'All' },
	{ value: 'native', label: 'Native' },
	{ value: 'device', label: 'Device-synced' },
]

/** `since` choices, as whole days back from now. */
export const SINCE_OPTIONS = [
	{ value: '7', label: 'Last 7 days' },
	{ value: '30', label: 'Last 30 days' },
	{ value: '90', label: 'Last 90 days' },
	{ value: '365', label: 'Last year' },
]
/** Sentinel for a select whose "no filter" entry cannot be an empty string. */
export const ANY_OPTION = '__any__'

/** The hub's filters, as the URL-friendly values the page keeps in its query. */
export type AnnotationFacets = {
	kind: AnnotationKind | null
	/** The coarse cut; `source` wins over it when both are set */
	lane: AnnotationLane | null
	source: AnnotationSourceKind | null
	/** A durable registered device, by its device id. */
	sourceDeviceId: string | null
	/** A book, by media id */
	book: string | null
	/** Whole days back from now */
	sinceDays: string | null
	search: string | null
}

export const NO_FACETS: AnnotationFacets = {
	kind: null,
	lane: null,
	source: null,
	sourceDeviceId: null,
	book: null,
	sinceDays: null,
	search: null,
}

/** Which lane chip is active: a single source implies its lane. */
export function activeLane(facets: AnnotationFacets): AnnotationLane | 'all' {
	if (facets.source) return facets.source === 'WEB' ? 'native' : 'device'
	return facets.lane ?? 'all'
}

export function annotationFilter(facets: AnnotationFacets): Record<string, unknown> {
	const days = facets.sinceDays ? Number(facets.sinceDays) : null
	const source = facets.source
		? [facets.source]
		: facets.lane === 'native'
			? ['WEB']
			: facets.lane === 'device'
				? DEVICE_SOURCES
				: null
	return {
		...(facets.kind ? { kind: [facets.kind] } : {}),
		...(source ? { source } : {}),
		...(facets.sourceDeviceId ? { sourceDeviceId: facets.sourceDeviceId } : {}),
		...(facets.book ? { mediaId: facets.book } : {}),
		...(facets.search ? { query: facets.search } : {}),
		...(days && Number.isFinite(days)
			? { since: new Date(Date.now() - days * 86_400_000).toISOString() }
			: {}),
	}
}

export function activeFacets(
	facets: AnnotationFacets,
	bookTitle: (mediaId: string) => string,
	deviceLabel: (deviceId: string) => string = (deviceId) => deviceId,
): { key: keyof AnnotationFacets; label: string }[] {
	const chips: { key: keyof AnnotationFacets; label: string }[] = []
	if (facets.kind) chips.push({ key: 'kind', label: KIND_LABELS[facets.kind] })
	if (facets.source) chips.push({ key: 'source', label: SOURCE_LABELS[facets.source] })
	if (facets.sourceDeviceId) {
		chips.push({ key: 'sourceDeviceId', label: `Device: ${deviceLabel(facets.sourceDeviceId)}` })
	}
	if (facets.book) chips.push({ key: 'book', label: bookTitle(facets.book) })
	if (facets.sinceDays) {
		const option = SINCE_OPTIONS.find((candidate) => candidate.value === facets.sinceDays)
		chips.push({ key: 'sinceDays', label: option?.label ?? `Last ${facets.sinceDays} days` })
	}
	if (facets.search) chips.push({ key: 'search', label: `“${facets.search}”` })
	return chips
}

/** One book with the annotations that belong to it, in arrival order. */
export type AnnotationGroup = {
	book: ConsoleAnnotationFieldsFragment['book']
	items: ConsoleAnnotationFieldsFragment[]
}

/**
 * The server returns book-contiguous items (books newest-first, annotations
 * inside a book in the order they were made), so grouping is a single pass
 * that preserves both orders.
 */
export function groupByBook(items: ConsoleAnnotationFieldsFragment[]): AnnotationGroup[] {
	const groups: AnnotationGroup[] = []
	for (const item of items) {
		const last = groups.at(-1)
		if (last && last.book.key === item.book.key) last.items.push(item)
		else groups.push({ book: item.book, items: [item] })
	}
	return groups
}

/**
 * The reader's deep-link query for an annotation's anchor. The reader opens at
 * it once and only writes progress after the reader actually moves, so
 * following a link never rewrites the book's position.
 */
export function readerAnchor(annotation: ConsoleAnnotationFieldsFragment): string {
	const params = new URLSearchParams()
	if (annotation.href) params.set('href', annotation.href)
	if (annotation.fragment) params.set('fragment', annotation.fragment)
	if (typeof annotation.progression === 'number') {
		params.set('progression', String(annotation.progression))
	}
	if (typeof annotation.page === 'number') params.set('page', String(annotation.page))
	const query = params.toString()
	return query ? `?${query}` : ''
}

/** `Chapter Three · page 42 · 62%` — whatever the anchor actually carries. */
export function anchorLabel(annotation: ConsoleAnnotationFieldsFragment): string | null {
	const { progression } = annotation
	const percent =
		typeof progression === 'number' && Number.isFinite(progression)
			? `${Math.round(Math.min(1, Math.max(0, progression)) * 100)}%`
			: null
	const parts = [
		annotation.chapterTitle,
		typeof annotation.page === 'number' ? `page ${annotation.page}` : null,
		percent,
	].filter((part): part is string => Boolean(part))
	return parts.length ? parts.join(' · ') : null
}
