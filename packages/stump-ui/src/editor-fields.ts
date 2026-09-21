/**
 * Canonical metadata editor vocabulary and pure field behavior.
 *
 * The editor and Home adapters translate their GraphQL records into this
 * vocabulary. Keeping values, locks, validation, candidate comparison and
 * serialization here prevents the two applications from drifting.
 */
export type SharedPublicationKind = 'book' | 'audiobook'
export type SharedEditorMode = 'item' | 'media'
export type SharedFieldKind = 'text' | 'textarea' | 'list' | 'integer' | 'decimal' | 'date'
export type SharedFieldLane = 'ingest' | 'media'
export type SharedFieldGroup =
	| 'identity'
	| 'series'
	| 'people'
	| 'publication'
	| 'description'
	| 'classification'
	| 'identifiers'
	| 'credits'
	| 'extras'

export type SharedIngestKey =
	| 'TITLE'
	| 'SORT_TITLE'
	| 'SERIES'
	| 'SERIES_INDEX'
	| 'AUTHORS'
	| 'NARRATORS'
	| 'PUBLISHER'
	| 'PUBLISHED_DATE'
	| 'LANGUAGE'
	| 'SUMMARY'
	| 'TAGS'
	| 'GENRES'
	| 'ISBN'
	| 'IDENTIFIERS'
	| 'AGE_RATING'
	| 'PAGE_COUNT'
	| 'COVER_URL'
	| 'STATUS'

export type SharedFieldKey =
	| 'title'
	| 'titleSort'
	| 'series'
	| 'number'
	| 'writers'
	| 'narrators'
	| 'publisher'
	| 'releaseDate'
	| 'language'
	| 'pageCount'
	| 'ageRating'
	| 'summary'
	| 'genres'
	| 'tags'
	| 'isbn'
	| 'identifierMobiAsin'
	| 'identifierAmazon'
	| 'identifierGoogle'
	| 'identifierCalibre'
	| 'identifierUuid'
	| 'volume'
	| 'seriesGroup'
	| 'storyArc'
	| 'storyArcNumber'
	| 'format'
	| 'notes'
	| 'characters'
	| 'teams'
	| 'links'
	| 'pencillers'
	| 'inkers'
	| 'colorists'
	| 'letterers'
	| 'coverArtists'
	| 'editors'

export type SharedFieldDescriptor = {
	key: SharedFieldKey
	label: string
	kind: SharedFieldKind
	group: SharedFieldGroup
	lane: SharedFieldLane
	applies: 'all' | SharedPublicationKind
	/** Public lock/policy field name. `null` means the storage column has no public field pick. */
	field: string | null
	ingestKey?: SharedIngestKey
	lockAliases?: string[]
	placeholder?: string
	hint?: string
}

export const SHARED_EDITOR_FIELD_GROUPS: { id: SharedFieldGroup; label: string }[] = [
	{ id: 'identity', label: 'Title' },
	{ id: 'series', label: 'Series' },
	{ id: 'people', label: 'People' },
	{ id: 'publication', label: 'Publication' },
	{ id: 'description', label: 'Description' },
	{ id: 'classification', label: 'Classification' },
	{ id: 'identifiers', label: 'Identifiers' },
	{ id: 'credits', label: 'Comic credits' },
	{ id: 'extras', label: 'More' },
]

export const SHARED_EDITOR_FIELDS: readonly SharedFieldDescriptor[] = [
	{
		key: 'title',
		label: 'Title',
		kind: 'text',
		group: 'identity',
		lane: 'ingest',
		applies: 'all',
		field: 'TITLE',
		ingestKey: 'TITLE',
	},
	{
		key: 'titleSort',
		label: 'Sort title',
		kind: 'text',
		group: 'identity',
		lane: 'ingest',
		applies: 'all',
		field: 'TITLE_SORT',
		ingestKey: 'SORT_TITLE',
		hint: 'How the title sorts on a shelf, for example "Hobbit, The".',
	},
	{
		key: 'series',
		label: 'Series',
		kind: 'text',
		group: 'series',
		lane: 'ingest',
		applies: 'all',
		field: 'SERIES',
		ingestKey: 'SERIES',
	},
	{
		key: 'number',
		label: 'Number in series',
		kind: 'decimal',
		group: 'series',
		lane: 'ingest',
		applies: 'all',
		field: 'NUMBER',
		ingestKey: 'SERIES_INDEX',
		placeholder: '1',
	},
	{
		key: 'volume',
		label: 'Volume',
		kind: 'integer',
		group: 'series',
		lane: 'media',
		applies: 'book',
		field: null,
	},
	{
		key: 'seriesGroup',
		label: 'Series group',
		kind: 'text',
		group: 'series',
		lane: 'media',
		applies: 'book',
		field: 'SERIES_GROUP',
	},
	{
		key: 'storyArc',
		label: 'Story arc',
		kind: 'text',
		group: 'series',
		lane: 'media',
		applies: 'book',
		field: 'STORY_ARC',
	},
	{
		key: 'storyArcNumber',
		label: 'Story arc number',
		kind: 'decimal',
		group: 'series',
		lane: 'media',
		applies: 'book',
		field: 'STORY_ARC_NUMBER',
	},
	{
		key: 'writers',
		label: 'Authors',
		kind: 'list',
		group: 'people',
		lane: 'ingest',
		applies: 'all',
		field: 'WRITERS',
		ingestKey: 'AUTHORS',
		lockAliases: [
			'ARTISTS',
			'EDITORS',
			'INKERS',
			'LETTERERS',
			'COLORISTS',
			'COVER_ARTISTS',
			'PENCILLERS',
			'TEAMS',
		],
		placeholder: 'Comma separated',
	},
	{
		key: 'narrators',
		label: 'Narrators',
		kind: 'list',
		group: 'people',
		lane: 'ingest',
		applies: 'audiobook',
		field: 'NARRATORS',
		ingestKey: 'NARRATORS',
		placeholder: 'Comma separated',
		hint: 'The readers of this edition, a separate credit from the authors.',
	},
	{
		key: 'publisher',
		label: 'Publisher',
		kind: 'text',
		group: 'publication',
		lane: 'ingest',
		applies: 'all',
		field: 'PUBLISHER',
		ingestKey: 'PUBLISHER',
		lockAliases: ['IMPRINT'],
	},
	{
		key: 'releaseDate',
		label: 'Published',
		kind: 'date',
		group: 'publication',
		lane: 'ingest',
		applies: 'all',
		field: 'RELEASE_DATE',
		ingestKey: 'PUBLISHED_DATE',
		lockAliases: ['YEAR'],
		placeholder: 'YYYY, YYYY-MM, or YYYY-MM-DD',
	},
	{
		key: 'language',
		label: 'Language',
		kind: 'text',
		group: 'publication',
		lane: 'ingest',
		applies: 'all',
		field: 'LANGUAGE',
		ingestKey: 'LANGUAGE',
		placeholder: 'en',
	},
	{
		key: 'pageCount',
		label: 'Page count',
		kind: 'integer',
		group: 'publication',
		lane: 'ingest',
		applies: 'book',
		field: 'PAGE_COUNT',
		ingestKey: 'PAGE_COUNT',
	},
	{
		key: 'format',
		label: 'Format',
		kind: 'text',
		group: 'publication',
		lane: 'media',
		applies: 'book',
		field: 'FORMAT',
	},
	{
		key: 'summary',
		label: 'Description',
		kind: 'textarea',
		group: 'description',
		lane: 'ingest',
		applies: 'all',
		field: 'SUMMARY',
		ingestKey: 'SUMMARY',
	},
	{
		key: 'notes',
		label: 'Notes',
		kind: 'textarea',
		group: 'description',
		lane: 'media',
		applies: 'all',
		field: 'NOTES',
	},
	{
		key: 'genres',
		label: 'Genres',
		kind: 'list',
		group: 'classification',
		lane: 'ingest',
		applies: 'all',
		field: 'GENRES',
		ingestKey: 'GENRES',
		placeholder: 'Comma separated',
	},
	{
		key: 'tags',
		label: 'Tags',
		kind: 'list',
		group: 'classification',
		lane: 'ingest',
		applies: 'all',
		field: 'TAGS',
		ingestKey: 'TAGS',
		placeholder: 'Comma separated',
	},
	{
		key: 'ageRating',
		label: 'Age rating',
		kind: 'integer',
		group: 'classification',
		lane: 'ingest',
		applies: 'all',
		field: 'AGE_RATING',
		ingestKey: 'AGE_RATING',
		placeholder: 'Minimum age',
	},
	{
		key: 'isbn',
		label: 'ISBN',
		kind: 'text',
		group: 'identifiers',
		lane: 'ingest',
		applies: 'all',
		field: 'ISBN',
		ingestKey: 'ISBN',
	},
	{
		key: 'identifierMobiAsin',
		label: 'ASIN',
		kind: 'text',
		group: 'identifiers',
		lane: 'media',
		applies: 'all',
		field: 'IDENTIFIER_MOBI_ASIN',
	},
	{
		key: 'identifierAmazon',
		label: 'Amazon',
		kind: 'text',
		group: 'identifiers',
		lane: 'media',
		applies: 'all',
		field: 'IDENTIFIER_AMAZON',
	},
	{
		key: 'identifierGoogle',
		label: 'Google Books',
		kind: 'text',
		group: 'identifiers',
		lane: 'media',
		applies: 'all',
		field: 'IDENTIFIER_GOOGLE',
	},
	{
		key: 'identifierCalibre',
		label: 'Calibre',
		kind: 'text',
		group: 'identifiers',
		lane: 'media',
		applies: 'all',
		field: 'IDENTIFIER_CALIBRE',
	},
	{
		key: 'identifierUuid',
		label: 'UUID',
		kind: 'text',
		group: 'identifiers',
		lane: 'media',
		applies: 'all',
		field: 'IDENTIFIER_UUID',
	},
	{
		key: 'pencillers',
		label: 'Pencillers',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'PENCILLERS',
		placeholder: 'Comma separated',
	},
	{
		key: 'inkers',
		label: 'Inkers',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'INKERS',
		placeholder: 'Comma separated',
	},
	{
		key: 'colorists',
		label: 'Colorists',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'COLORISTS',
		placeholder: 'Comma separated',
	},
	{
		key: 'letterers',
		label: 'Letterers',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'LETTERERS',
		placeholder: 'Comma separated',
	},
	{
		key: 'coverArtists',
		label: 'Cover artists',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'COVER_ARTISTS',
		placeholder: 'Comma separated',
	},
	{
		key: 'editors',
		label: 'Editors',
		kind: 'list',
		group: 'credits',
		lane: 'media',
		applies: 'book',
		field: 'EDITORS',
		placeholder: 'Comma separated',
	},
	{
		key: 'characters',
		label: 'Characters',
		kind: 'list',
		group: 'extras',
		lane: 'media',
		applies: 'all',
		field: 'CHARACTERS',
		placeholder: 'Comma separated',
	},
	{
		key: 'teams',
		label: 'Teams',
		kind: 'list',
		group: 'extras',
		lane: 'media',
		applies: 'book',
		field: 'TEAMS',
		placeholder: 'Comma separated',
	},
	{
		key: 'links',
		label: 'Links',
		kind: 'list',
		group: 'extras',
		lane: 'media',
		applies: 'all',
		field: 'LINKS',
		placeholder: 'Comma separated URLs',
	},
]

export const SHARED_EDITOR_EVIDENCE_ONLY_KEYS: { key: SharedIngestKey; label: string }[] = [
	{ key: 'COVER_URL', label: 'Cover' },
	{ key: 'IDENTIFIERS', label: 'Provider identity' },
	{ key: 'STATUS', label: 'Series status' },
]

/** Short aliases keep adapter imports readable while the SHARED names remain the documented exports. */
export type FieldDescriptor = SharedFieldDescriptor
export type FieldKey = SharedFieldKey
export type PublicationKind = SharedPublicationKind
export type FieldKind = SharedFieldKind
export type FieldLane = SharedFieldLane
export type FieldGroup = SharedFieldGroup
export type IngestKey = SharedIngestKey
export const FIELD_GROUPS = SHARED_EDITOR_FIELD_GROUPS
export const FIELDS = SHARED_EDITOR_FIELDS
export const EVIDENCE_ONLY_KEYS = SHARED_EDITOR_EVIDENCE_ONLY_KEYS

export const FIELD_BY_KEY: Record<SharedFieldKey, SharedFieldDescriptor> = Object.fromEntries(
	FIELDS.map((descriptor) => [descriptor.key, descriptor]),
) as Record<SharedFieldKey, SharedFieldDescriptor>

export const FIELD_BY_INGEST_KEY: Partial<Record<SharedIngestKey, SharedFieldDescriptor>> =
	Object.fromEntries(
		FIELDS.filter((descriptor) => descriptor.ingestKey).map((descriptor) => [
			descriptor.ingestKey,
			descriptor,
		]),
	)

/** The fields one record shows: a staged item edits the ingest lane only; a library record edits every column. */
export function visibleFields(
	mode: SharedEditorMode,
	kind: SharedPublicationKind,
): SharedFieldDescriptor[] {
	return FIELDS.filter(
		(descriptor) =>
			(mode === 'media' || descriptor.lane === 'ingest') &&
			(descriptor.applies === 'all' || descriptor.applies === kind),
	)
}

export type Draft = Record<SharedFieldKey, string>
export type SharedDraft = Draft

export function emptyDraft(): Draft {
	return Object.fromEntries(FIELDS.map((descriptor) => [descriptor.key, ''])) as Draft
}

function joinList(values: readonly string[]): string {
	return values
		.map((value) => value.trim())
		.filter(Boolean)
		.join(', ')
}

export function splitList(text: string): string[] {
	return text
		.split(',')
		.map((value) => value.trim())
		.filter(Boolean)
}

function formatDate(
	year: number | string | null | undefined,
	month: number | string | null | undefined,
	day: number | string | null | undefined,
): string {
	if (year === null || year === undefined || year === '') return ''
	const parts = [String(year).padStart(4, '0')]
	if (month !== null && month !== undefined && month !== '') {
		parts.push(String(month).padStart(2, '0'))
		if (day !== null && day !== undefined && day !== '') parts.push(String(day).padStart(2, '0'))
	}
	return parts.join('-')
}

/** Safely reads JSON scalars returned by a GraphQL JSON field. */
export function parseJsonObject(value: unknown): Record<string, unknown> {
	if (typeof value === 'string') {
		try {
			const parsed: unknown = JSON.parse(value)
			return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
				? (parsed as Record<string, unknown>)
				: {}
		} catch {
			return {}
		}
	}
	return value && typeof value === 'object' && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {}
}

/** The text an ingest-vocabulary JSON value (candidate field or staged value) is edited as. */
export function textFromIngestValue(kind: SharedFieldKind, value: unknown): string {
	if (value === null || value === undefined) return ''
	if (Array.isArray(value))
		return joinList(
			value.map((entry) => (typeof entry === 'string' ? entry : JSON.stringify(entry))),
		)
	if (kind === 'date' && typeof value === 'object') {
		const date = value as {
			year?: number | string | null
			month?: number | string | null
			day?: number | string | null
		}

		return formatDate(date.year, date.month, date.day)
	}
	if (typeof value === 'object') {
		return Object.entries(value as Record<string, unknown>)
			.map(([key, entry]) => `${key}: ${typeof entry === 'string' ? entry : JSON.stringify(entry)}`)
			.join(', ')
	}
	if (kind === 'list' && typeof value === 'string') return joinList(value.split(','))
	return typeof value === 'string' ? value : String(value)
}
export function publicationKindOfItem(item: {
	mediaType: string
	audio?: unknown | null
}): SharedPublicationKind {
	return item.mediaType === 'AUDIO' || Boolean(item.audio) ? 'audiobook' : 'book'
}

export function publicationKindOfMedia(media: { audio?: unknown | null }): SharedPublicationKind {
	return media.audio ? 'audiobook' : 'book'
}

/** Two texts describe the same value when they agree after list and whitespace normalisation. */
export function normalizeText(kind: SharedFieldKind, text: string): string {
	if (kind === 'list') return joinList(splitList(text))
	if (kind === 'decimal' || kind === 'integer') {
		const trimmed = text.trim()
		const parsed = Number(trimmed)
		return trimmed && Number.isFinite(parsed) ? String(parsed) : trimmed
	}
	return text.trim()
}

export function sameValue(kind: SharedFieldKind, left: string, right: string): boolean {
	return normalizeText(kind, left) === normalizeText(kind, right)
}

/** GraphQL/storage key for the canonical text draft. */
export function sharedFieldStorageKey(key: SharedFieldKey): string {
	return key === 'isbn' ? 'identifierIsbn' : key
}

function metadataValue(record: Record<string, unknown>, key: SharedFieldKey): unknown {
	const camelKey = sharedFieldStorageKey(key)
	if (Object.hasOwn(record, camelKey)) return record[camelKey]
	const snakeKey = camelKey.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`)
	return record[snakeKey]
}

/** Builds a draft from a MediaMetadata-like record and optional media tags. */
export function draftFromMetadata(
	source: Record<string, unknown> | null | undefined,
	tags?: readonly unknown[],
): Draft {
	const draft = emptyDraft()
	const record = source ?? {}
	for (const descriptor of FIELDS) {
		if (descriptor.key === 'releaseDate') {
			draft[descriptor.key] = formatDate(
				(record.year ?? record.releaseDate ?? record.release_date) as
					| number
					| string
					| null
					| undefined,
				(record.month ?? record.releaseMonth ?? record.release_month) as
					| number
					| string
					| null
					| undefined,
				(record.day ?? record.releaseDay ?? record.release_day) as
					| number
					| string
					| null
					| undefined,
			)
			continue
		}
		const value = descriptor.key === 'tags' && tags ? tags : metadataValue(record, descriptor.key)
		draft[descriptor.key] = textFromIngestValue(descriptor.kind, value)
	}
	return draft
}

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

/** Whether a public lock list locks this field, counting every spelling that folds onto it. */
export function isLocked(
	descriptor: SharedFieldDescriptor,
	lockedFields: readonly string[],
): boolean {
	if (!descriptor.field) return false
	if (lockedFields.includes(descriptor.field)) return true
	return (descriptor.lockAliases ?? []).some((alias) => lockedFields.includes(alias))
}

/** The lock list with one field toggled, written in the field's canonical public spelling. */
export function toggleLock(
	descriptor: SharedFieldDescriptor,
	lockedFields: readonly string[],
): string[] {
	if (!descriptor.field) return [...lockedFields]
	const fold = [descriptor.field, ...(descriptor.lockAliases ?? [])]
	const without = lockedFields.filter((field) => !fold.includes(field))
	return isLocked(descriptor, lockedFields) ? without : [...without, descriptor.field]
}

// ---------------------------------------------------------------------------
// Validation and serialization
// ---------------------------------------------------------------------------

const DATE_PATTERN = /^\d{4}(-(0[1-9]|1[0-2])(-(0[1-9]|[12]\d|3[01]))?)?$/

/** A human message when the text is not a value of the field's kind; `null` when it is (or empty). */
export function validateText(descriptor: SharedFieldDescriptor, text: string): string | null {
	const trimmed = text.trim()
	if (!trimmed) return null
	switch (descriptor.kind) {
		case 'integer':
			return /^-?\d+$/.test(trimmed) ? null : `${descriptor.label} must be a whole number.`
		case 'decimal':
			return Number.isFinite(Number(trimmed)) ? null : `${descriptor.label} must be a number.`
		case 'date':
			return DATE_PATTERN.test(trimmed)
				? null
				: `${descriptor.label} must be YYYY, YYYY-MM, or YYYY-MM-DD.`
		default:
			return null
	}
}

/** The JSON value a MANUAL pick carries for this field. */
export function ingestValueFromText(descriptor: SharedFieldDescriptor, text: string): unknown {
	const trimmed = text.trim()
	switch (descriptor.kind) {
		case 'list':
			return splitList(trimmed)
		case 'integer':
			return Number.parseInt(trimmed, 10)
		case 'decimal':
			return Number(trimmed)
		default:
			return trimmed
	}
}

function optionalText(text: string): string | null {
	const trimmed = text.trim()
	return trimmed ? trimmed : null
}

function optionalInteger(text: string): number | null {
	const trimmed = text.trim()
	return trimmed ? Number.parseInt(trimmed, 10) : null
}

function optionalList(text: string): string[] | null {
	const values = splitList(text)
	return values.length ? values : null
}

/** Complete metadata input shared by Home and the media editor. */
export function metadataInputFromDraft(draft: Draft): Record<string, unknown> {
	const date = draft.releaseDate.trim().split('-')
	const year = date[0] ? Number.parseInt(date[0], 10) : null
	const month = date[1] ? Number.parseInt(date[1], 10) : null
	const day = date[2] ? Number.parseInt(date[2], 10) : null
	return {
		title: optionalText(draft.title),
		titleSort: optionalText(draft.titleSort),
		series: optionalText(draft.series),
		number: optionalText(draft.number),
		volume: optionalInteger(draft.volume),
		seriesGroup: optionalText(draft.seriesGroup),
		storyArc: optionalText(draft.storyArc),
		storyArcNumber: optionalText(draft.storyArcNumber),
		writers: optionalList(draft.writers),
		narrators: optionalList(draft.narrators),
		publisher: optionalText(draft.publisher),
		year: Number.isFinite(year) ? year : null,
		month: Number.isFinite(month) ? month : null,
		day: Number.isFinite(day) ? day : null,
		language: optionalText(draft.language),
		pageCount: optionalInteger(draft.pageCount),
		format: optionalText(draft.format),
		summary: optionalText(draft.summary),
		notes: optionalText(draft.notes),
		genres: optionalList(draft.genres),
		ageRating: optionalInteger(draft.ageRating),
		identifierIsbn: optionalText(draft.isbn),
		identifierMobiAsin: optionalText(draft.identifierMobiAsin),
		identifierAmazon: optionalText(draft.identifierAmazon),
		identifierGoogle: optionalText(draft.identifierGoogle),
		identifierCalibre: optionalText(draft.identifierCalibre),
		identifierUuid: optionalText(draft.identifierUuid),
		pencillers: optionalList(draft.pencillers),
		inkers: optionalList(draft.inkers),
		colorists: optionalList(draft.colorists),
		letterers: optionalList(draft.letterers),
		coverArtists: optionalList(draft.coverArtists),
		editors: optionalList(draft.editors),
		characters: optionalList(draft.characters),
		teams: optionalList(draft.teams),
		links: optionalList(draft.links),
	}
}

// ---------------------------------------------------------------------------
// Candidates and save plans
// ---------------------------------------------------------------------------

export interface SharedCandidateLike {
	id: string
	provider: string
	providerVersion?: string | null
	model?: string | null
	confidence: number
	fields: unknown
	fieldConfidences?: unknown
	status?: string | null
	createdAt?: string | null
	/** External search results have no ingest candidate id and must save as manual values. */
	source?: 'persisted' | 'external'
}

export type CandidateLike = SharedCandidateLike

export interface CandidateSource {
	candidateId: string
	provider: string
	/** Candidate text for this field; a later edit turns the pick manual. */
	text: string
}

export type Sources = Partial<Record<SharedFieldKey, CandidateSource>>
export type SharedSources = Sources

export interface CandidateOffer {
	descriptor: SharedFieldDescriptor
	text: string
	confidence: number | null
	applied: boolean
	fills: boolean
}

export interface CandidateEvidence {
	key: SharedIngestKey
	label: string
	text: string
	url: string | null
}

/** One candidate's fields, split into applicable offers and evidence-only rows, in form order. */
export function candidateOffers(
	candidate: SharedCandidateLike,
	fields: readonly SharedFieldDescriptor[],
	draft: Draft,
): { offers: CandidateOffer[]; evidence: CandidateEvidence[] } {
	const values = parseJsonObject(candidate.fields)
	const confidences = parseJsonObject(candidate.fieldConfidences)
	const offers: CandidateOffer[] = []
	for (const descriptor of fields) {
		if (!descriptor.ingestKey || !Object.hasOwn(values, descriptor.ingestKey)) continue
		const text = textFromIngestValue(descriptor.kind, values[descriptor.ingestKey])
		if (!text) continue
		const confidence = confidences[descriptor.ingestKey]
		offers.push({
			descriptor,
			text,
			confidence: typeof confidence === 'number' ? confidence : null,
			applied: sameValue(descriptor.kind, draft[descriptor.key], text),
			fills: !draft[descriptor.key].trim(),
		})
	}
	const evidence: CandidateEvidence[] = []
	for (const entry of SHARED_EDITOR_EVIDENCE_ONLY_KEYS) {
		if (!Object.hasOwn(values, entry.key)) continue
		const value = values[entry.key]
		const text = textFromIngestValue('text', value)
		if (!text) continue
		evidence.push({
			key: entry.key,
			label: entry.label,
			text,
			url:
				entry.key === 'COVER_URL' && typeof value === 'string' && /^https?:\/\//.test(value)
					? value
					: null,
		})
	}
	return { offers, evidence }
}

export function changedFields(
	draft: Draft,
	baseline: Draft,
	fields: readonly SharedFieldDescriptor[],
): SharedFieldKey[] {
	return fields
		.filter(
			(descriptor) => !sameValue(descriptor.kind, draft[descriptor.key], baseline[descriptor.key]),
		)
		.map((descriptor) => descriptor.key)
}

export interface SharedSavePlan {
	mode: SharedEditorMode
	fields: readonly SharedFieldDescriptor[]
	draft: Draft
	baseline: Draft
	sources: Sources
	changed: SharedFieldKey[]
	skippedLocked: SharedFieldKey[]
	metadata: Record<string, unknown>
}

export function buildSharedSavePlan(options: {
	mode: SharedEditorMode
	fields: readonly SharedFieldDescriptor[]
	draft: Draft
	baseline: Draft
	sources: Sources
	lockedFields: readonly string[]
}): SharedSavePlan {
	const { mode, fields, draft, baseline, sources, lockedFields } = options
	const changed: SharedFieldKey[] = []
	const skippedLocked: SharedFieldKey[] = []
	for (const descriptor of fields) {
		if (sameValue(descriptor.kind, draft[descriptor.key], baseline[descriptor.key])) continue
		if (mode === 'media' && isLocked(descriptor, lockedFields)) skippedLocked.push(descriptor.key)
		else changed.push(descriptor.key)
	}
	return {
		mode,
		fields,
		draft,
		baseline,
		sources,
		changed,
		skippedLocked,
		metadata: metadataInputFromDraft(draft),
	}
}

/** Candidates grouped by provider, best confidence first within and across groups. */
export function groupByProvider<T extends SharedCandidateLike>(
	candidates: readonly T[],
): { provider: string; candidates: T[] }[] {
	const groups = new Map<string, T[]>()
	for (const candidate of candidates) {
		const list = groups.get(candidate.provider) ?? []
		list.push(candidate)
		groups.set(candidate.provider, list)
	}
	return [...groups.entries()]
		.map(([provider, list]) => ({
			provider,
			candidates: [...list].sort(
				(left, right) =>
					right.confidence - left.confidence ||
					(right.createdAt ?? '').localeCompare(left.createdAt ?? ''),
			),
		}))
		.sort(
			(left, right) =>
				(right.candidates[0]?.confidence ?? 0) - (left.candidates[0]?.confidence ?? 0) ||
				left.provider.localeCompare(right.provider),
		)
}

/** Provider ids as humans read them: registry ids are lowercase slugs. */
export function providerLabel(id: string): string {
	const KNOWN: Record<string, string> = {
		embedded: 'Embedded metadata',
		llm: 'AI enrichment',
		hardcover: 'Hardcover',
		openlibrary: 'Open Library',
		'open-library': 'Open Library',
		googlebooks: 'Google Books',
		'google-books': 'Google Books',
		audible: 'Audible',
		audnexus: 'Audnexus',
		anilist: 'AniList',
		mangadex: 'MangaDex',
		mangaupdates: 'MangaUpdates',
		comicvine: 'Comic Vine',
		metron: 'Metron',
	}
	return KNOWN[id] ?? id.replace(/[-_]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
}
