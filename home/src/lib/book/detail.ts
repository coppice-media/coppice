export type EditionKind = 'EBOOK' | 'AUDIOBOOK' | 'OTHER'
export type FileStatus = 'UNKNOWN' | 'READY' | 'UNSUPPORTED' | 'ERROR' | 'MISSING' | string

export type BookMetadata = Record<string, unknown> & {
	title?: string | null
	writers?: string[] | null
	narrators?: string[] | null
	publisher?: string | null
	summary?: string | null
	genres?: string[] | null
	lockedFields?: string[] | null
}

export type BookAudio = {
	durationMs: number
	codec: string
	sampleRate?: number | null
	channels?: number | null
	bitrate?: number | null
	chapterSource: string
	chapters: {
		id?: string
		index: number
		title?: string | null
		startMs: number
		endMs?: number | null
	}[]
}

export type BookFile = {
	path: string
	size: number
	extension: string
	hash?: string | null
	koreaderHash?: string | null
	status: FileStatus
	modifiedAt?: string | null
}

export type BookEdition = {
	mediaId: string
	kind: EditionKind
	title: string
	metadata?: BookMetadata | null
	file: BookFile
	audio?: BookAudio | null
	pairEvidence?: string | null
}

export type BookMismatch = {
	field: string
	ebookValue?: string | null
	audiobookValue?: string | null
	workValue?: string | null
	resolved: boolean
}

export type BookWorkMetadata = {
	workId: string
	title?: string | null
	author?: string | null
	metadata?: Record<string, unknown> | null
	lockedFields?: string[] | null
}

export type BookReview = {
	id: string
	rating: number
	content?: string | null
	isPrivate: boolean
	mediaId?: string | null
	workId?: string | null
	createdAt?: string | null
	updatedAt?: string | null
}

export type ReadAloud = {
	status:
		| 'NO_AUDIOBOOK_PAIRED'
		| 'UNAVAILABLE'
		| 'CHAPTER_MAP_UNAVAILABLE'
		| 'SYNC_MAP_UNAVAILABLE'
		| 'CACHE_MISSING'
		| 'READY'
		| 'FAILED'
		| string
	reason?: string | null
	ebookMediaId?: string | null
	audioMediaId?: string | null
	chapterMap: {
		ebookSpineIndex: number
		audioChapterIndex: number
		confidence: number
		ebookMediaId: string
		audioMediaId: string
	}[]
	syncMap?: {
		id: string
		granularity: string
		generator: string
		generatorVersion: string
		cueCount: number
		coverage?: number | null
		source: string
		createdAt: string
	} | null
	artifact?: { url: string; mimeType: string; cacheKey: string } | null
}

export type BookDetail = {
	mediaId: string
	workId?: string | null
	title: string
	authors: string[]
	workMetadata?: BookWorkMetadata | null
	editions: BookEdition[]
	mismatches: BookMismatch[]
	review?: BookReview | null
	readAloud?: ReadAloud | null
}

export type BookCandidate = {
	provider: string
	externalId: string
	confidence: number
	metadata?: Record<string, unknown> | null
}

export type BookReadingHead = {
	locator?: {
		chapterTitle?: string | null
		locations?: { progression?: number | null; totalProgression?: number | null } | null
	} | null
	progression?: number | null
	page?: number | null
	positionMs?: number | null
	trackIndex?: number | null
	completed: boolean
	updatedAt?: string | null
	sourceProtocol?: string | null
	sourceDeviceId?: string | null
	sourceDevice?: {
		id: string
		name?: string | null
		kind?: string | null
		revoked?: boolean
	} | null
}

export type BookReadingSession = {
	id: string
	sessionDate?: string | null
	status: string
	readthroughNumber: number
	startLocator?: unknown
	endLocator?: unknown
	startPage?: number | null
	endPage?: number | null
	endPositionMs?: number | null
	startPercentage?: number | null
	endPercentage?: number | null
	elapsedSeconds?: number | null
	notes?: string | null
	sourceProtocol?: string | null
	sourceDeviceIds: string[]
	sourceDevices: { id: string; name?: string | null; kind?: string | null; revoked?: boolean }[]
	liseurSessionId?: string | null
	createdAt?: string | null
	updatedAt?: string | null
}

export type BookLogEdition = {
	mediaId: string
	kind: EditionKind
	head?: BookReadingHead | null
	sessions: BookReadingSession[]
}

export type BookReadingLog = { mediaId: string; workId?: string | null; editions: BookLogEdition[] }

export type BookHighlight = {
	id: string
	kind: string
	source: string
	sourceDeviceId?: string | null
	sourceDeviceName?: string | null
	editable: boolean
	chapterTitle?: string | null
	href?: string | null
	fragment?: string | null
	page?: number | null
	progression?: number | null
	excerpt?: string | null
	note?: string | null
	color?: string | null
	createdAt?: string | null
	updatedAt?: string | null
	book?: { mediaId?: string | null; title: string } | null
}

export type SimilarBook = {
	mediaId: string
	workId?: string | null
	title: string
	authors: string[]
	kind: EditionKind
	score: number
}

export function formatBytes(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes < 0) return 'Unknown size'
	if (bytes < 1024) return `${bytes} B`
	const units = ['KiB', 'MiB', 'GiB', 'TiB']
	let value = bytes / 1024
	let unit = units[0]
	for (let index = 0; value >= 1024 && index < units.length - 1; index += 1) {
		value /= 1024
		unit = units[index + 1]
	}
	return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`
}

export function formatDuration(milliseconds: number | null | undefined): string {
	if (milliseconds == null || !Number.isFinite(milliseconds) || milliseconds < 0)
		return 'Unavailable'
	const totalSeconds = Math.round(milliseconds / 1000)
	const hours = Math.floor(totalSeconds / 3600)
	const minutes = Math.floor((totalSeconds % 3600) / 60)
	const seconds = totalSeconds % 60
	return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}:${String(seconds).padStart(2, '0')}`
}

export function formatDate(value: string | null | undefined): string {
	if (!value) return 'Unknown'
	const date = new Date(value)
	return Number.isNaN(date.valueOf())
		? value
		: new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' }).format(date)
}

export function labelKind(kind: EditionKind): string {
	return kind === 'EBOOK' ? 'Ebook' : kind === 'AUDIOBOOK' ? 'Audiobook' : 'Other edition'
}

export function statusLabel(status: FileStatus): string {
	return status === 'READY'
		? 'Ready'
		: status === 'MISSING'
			? 'Missing'
			: status === 'ERROR'
				? 'Error'
				: status === 'UNSUPPORTED'
					? 'Unsupported'
					: 'Unknown'
}

export function fieldLabel(field: string): string {
	return field
		.replaceAll('_', ' ')
		.toLowerCase()
		.replace(/\b\w/g, (letter) => letter.toUpperCase())
}

export function valueLabel(value: unknown): string {
	if (value == null || value === '') return 'Unavailable'
	if (Array.isArray(value)) return value.length ? value.join(', ') : 'Unavailable'
	if (typeof value === 'object') return 'Unavailable'
	return String(value)
}

export function normalizeSummary(value: unknown): string {
	if (typeof value !== 'string' || value.trim() === '') return ''

	return value
		.replace(/<\s*(script|style)\b[^>]*>[\s\S]*?<\s*\/\s*\1\s*>/gi, '')
		.replace(/<\s*br\s*\/?\s*>/gi, '\n')
		.replace(/<\s*\/\s*(p|div|li|h[1-6])\s*>/gi, '\n')
		.replace(/<[^>]+>/g, '')
		.replace(/&nbsp;|&#160;/gi, ' ')
		.replace(/&amp;/gi, '&')
		.replace(/&lt;/gi, '<')
		.replace(/&gt;/gi, '>')
		.replace(/&quot;|&#34;/gi, '"')
		.replace(/&#39;|&apos;/gi, "'")
		.replace(/[ \t]+\n/g, '\n')
		.replace(/\n{3,}/g, '\n\n')
		.trim()
}
