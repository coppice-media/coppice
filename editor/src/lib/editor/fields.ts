import type {
	EditorMediaQuery,
	IngestItemQuery,
	IngestMetadataFieldSelectionInput,
	MediaMetadataInput,
	MetadataField,
} from '$lib/graphql/generated/graphql'
import {
	FIELD_BY_INGEST_KEY as SHARED_FIELD_BY_INGEST_KEY,
	FIELD_BY_KEY as SHARED_FIELD_BY_KEY,
	FIELDS as SHARED_FIELDS,
	FIELD_GROUPS as SHARED_FIELD_GROUPS,
	EVIDENCE_ONLY_KEYS as SHARED_EVIDENCE_ONLY_KEYS,
	draftFromMetadata,
	buildSharedSavePlan,
	candidateOffers as sharedCandidateOffers,
	changedFields as sharedChangedFields,
	emptyDraft as sharedEmptyDraft,
	groupByProvider,
	ingestValueFromText,
	isLocked as sharedIsLocked,
	metadataInputFromDraft,
	normalizeText,
	parseJsonObject,
	providerLabel,
	publicationKindOfItem as _publicationKindOfItem,
	publicationKindOfMedia as _publicationKindOfMedia,
	sameValue,
	splitList,
	textFromIngestValue,
	toggleLock as sharedToggleLock,
	validateText,
	visibleFields as sharedVisibleFields,
	type CandidateEvidence,
	type CandidateLike,
	type CandidateOffer,
	type CandidateSource,
	type Draft,
	type FieldDescriptor as SharedFieldDescriptor,
	type FieldGroup,
	type FieldKey,
	type FieldKind,
	type FieldLane,
	type IngestKey,
	type PublicationKind,
	type SharedPublicationKind,
	type Sources,
} from '@stump/ui/editor-fields'

// This adapter is the only editor-app-specific layer. The shared package owns
// the field matrix, value normalization, locking, candidate comparison and
// serialization; GraphQL types and ingest save mutations stay here.
export type {
	CandidateEvidence,
	CandidateLike,
	CandidateOffer,
	CandidateSource,
	Draft,
	FieldGroup,
	FieldKey,
	FieldKind,
	FieldLane,
	IngestKey,
	PublicationKind,
	Sources,
}

export type FieldDescriptor = Omit<SharedFieldDescriptor, 'field' | 'lockAliases'> & {
	field: MetadataField | null
	lockAliases?: MetadataField[]
}

export const FIELD_GROUPS = SHARED_FIELD_GROUPS as { id: FieldGroup; label: string }[]
export const FIELDS = SHARED_FIELDS as unknown as readonly FieldDescriptor[]
export const FIELD_BY_KEY = SHARED_FIELD_BY_KEY as unknown as Record<FieldKey, FieldDescriptor>
export const FIELD_BY_INGEST_KEY = SHARED_FIELD_BY_INGEST_KEY as unknown as Partial<
	Record<IngestKey, FieldDescriptor>
>
export const EVIDENCE_ONLY_KEYS = SHARED_EVIDENCE_ONLY_KEYS as { key: IngestKey; label: string }[]

export function visibleFields(mode: 'item' | 'media', kind: PublicationKind): FieldDescriptor[] {
	return sharedVisibleFields(mode, kind as SharedPublicationKind) as unknown as FieldDescriptor[]
}

export function emptyDraft(): Draft {
	return sharedEmptyDraft()
}

export {
	groupByProvider,
	ingestValueFromText,
	normalizeText,
	parseJsonObject,
	providerLabel,
	sameValue,
	splitList,
	textFromIngestValue,
	validateText,
}

export type EditorMedia = NonNullable<EditorMediaQuery['mediaById']>
export type EditorItem = NonNullable<IngestItemQuery['ingestItem']>
export type EditorItemAudio = NonNullable<EditorItem['audio']>

/** The stored record of a committed book, one text per field. */
export function baselineFromMedia(media: EditorMedia): Draft {
	return draftFromMetadata(
		media.metadata as Record<string, unknown> | null | undefined,
		media.tags.map((tag) => tag.name),
	)
}

/** The current record of a staged item, including probe facts and pending picks. */
export function baselineFromItem(item: EditorItem): { draft: Draft; staged: Set<FieldKey> } {
	const draft = emptyDraft()
	const staged = new Set<FieldKey>()
	const audio = item.audio
	if (audio) {
		draft.title = audio.title ?? ''
		draft.writers = audio.author ?? ''
		draft.narrators = audio.narrator ?? ''
		draft.series = audio.album ?? ''
		draft.summary = audio.description ?? ''
		draft.genres = audio.genre ?? ''
		draft.releaseDate = audio.year === null || audio.year === undefined ? '' : String(audio.year)
	}
	const pending = parseJsonObject(item.pendingFields)
	for (const [key, value] of Object.entries(pending)) {
		const descriptor = FIELD_BY_INGEST_KEY[key as IngestKey]
		if (!descriptor) continue
		draft[descriptor.key] = textFromIngestValue(descriptor.kind, value)
		staged.add(descriptor.key)
	}
	return { draft, staged }
}

export function publicationKindOfItem(
	item: Pick<EditorItem, 'mediaType' | 'audio'>,
): PublicationKind {
	return _publicationKindOfItem(item)
}

export function publicationKindOfMedia(media: Pick<EditorMedia, 'audio'>): PublicationKind {
	return _publicationKindOfMedia(media)
}

export function isLocked(
	descriptor: FieldDescriptor,
	lockedFields: readonly MetadataField[],
): boolean {
	return sharedIsLocked(descriptor, lockedFields)
}

export function toggleLock(
	descriptor: FieldDescriptor,
	lockedFields: readonly MetadataField[],
): MetadataField[] {
	return sharedToggleLock(descriptor, lockedFields) as MetadataField[]
}

export interface SavePlan {
	/** `applyIngestMetadata` selections, one per changed ingest-lane field. */
	picks: IngestMetadataFieldSelectionInput[]
	/** `setMediaTags` payload when a library record's tags changed. */
	tags: string[] | null
	/** `updateMediaMetadata` payload when a media-lane column changed; always the complete draft. */
	mediaInput: MediaMetadataInput | null
	/** The fields the plan writes, for the summary line. */
	changed: FieldKey[]
	/** Locked fields the plan skipped. */
	skippedLocked: FieldKey[]
}

export function changedFields(
	draft: Draft,
	baseline: Draft,
	fields: readonly FieldDescriptor[],
): FieldKey[] {
	return sharedChangedFields(draft, baseline, fields) as FieldKey[]
}

/** Converts the shared plan into the standalone editor's ingest/media mutations. */
export function buildSavePlan(options: {
	mode: 'item' | 'media'
	fields: readonly FieldDescriptor[]
	draft: Draft
	baseline: Draft
	sources: Sources
	lockedFields: readonly MetadataField[]
}): SavePlan {
	const sharedPlan = buildSharedSavePlan(options)
	const picks: IngestMetadataFieldSelectionInput[] = []
	let tags: string[] | null = null
	let mediaChanged = false
	for (const descriptor of options.fields) {
		if (!sharedPlan.changed.includes(descriptor.key)) continue
		if (options.mode === 'media' && descriptor.key === 'tags') {
			tags = splitList(options.draft[descriptor.key])
			continue
		}
		if (descriptor.lane === 'media') {
			mediaChanged = true
			continue
		}
		const field = descriptor.field
		if (!field) continue
		const text = options.draft[descriptor.key]
		if (!text.trim()) {
			picks.push({ field, mode: 'CLEAR' })
			continue
		}
		const source = options.sources[descriptor.key]
		if (source && sameValue(descriptor.kind, source.text, text)) {
			picks.push({ field, mode: 'CANDIDATE', candidateId: source.candidateId })
			continue
		}
		picks.push({ field, mode: 'MANUAL', value: ingestValueFromText(descriptor, text) })
	}
	return {
		picks,
		tags,
		mediaInput: mediaChanged ? mediaInputFromDraft(options.draft) : null,
		changed: sharedPlan.changed as FieldKey[],
		skippedLocked: sharedPlan.skippedLocked as FieldKey[],
	}
}

export function mediaInputFromDraft(draft: Draft): MediaMetadataInput {
	return metadataInputFromDraft(draft) as MediaMetadataInput
}

export function candidateOffers(
	candidate: CandidateLike,
	fields: readonly FieldDescriptor[],
	draft: Draft,
): { offers: CandidateOffer[]; evidence: CandidateEvidence[] } {
	return sharedCandidateOffers(candidate, fields, draft)
}
