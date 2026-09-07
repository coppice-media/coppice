import type {
	IngestDropItemStatus,
	IngestMetadataFieldMode,
	MetadataField,
	Pagination
} from '$lib/graphql/generated/graphql';

export const METADATA_FIELDS: MetadataField[] = [
	'TITLE',
	'SUMMARY',
	'GENRES',
	'TAGS',
	'ARTISTS',
	'PUBLISHER',
	'YEAR',
	'AGE_RATING',
	'COVER',
	'STATUS',
	'VOLUME_COUNT',
	'PAGE_COUNT',
	'ISBN',
	'RELEASE_DATE',
	'COLORISTS',
	'LETTERERS',
	'COVER_ARTISTS',
	'WRITERS',
	'FORMAT',
	'TITLE_SORT',
	'NUMBER',
	'SERIES',
	'SERIES_GROUP',
	'NOTES',
	'LANGUAGE',
	'EDITORS',
	'INKERS',
	'TEAMS',
	'LINKS',
	'CHARACTERS',
	'STORY_ARC',
	'STORY_ARC_NUMBER',
	'BOOK_TYPE',
	'IMPRINT',
	'PUBLICATION_RUN',
	'PENCILLERS',
	'IDENTIFIER_AMAZON',
	'IDENTIFIER_CALIBRE',
	'IDENTIFIER_GOOGLE',
	'IDENTIFIER_MOBI_ASIN',
	'IDENTIFIER_UUID',
	'COMIC_ID',
	'META_TYPE',
	'COMIC_IMAGE',
	'DESCRIPTION_FORMATTED'
];

export const APPLYABLE_METADATA_FIELDS: Partial<Record<MetadataField, true>> = {
	TITLE: true,
	TITLE_SORT: true,
	SERIES: true,
	NUMBER: true,
	ARTISTS: true,
	PUBLISHER: true,
	RELEASE_DATE: true,
	LANGUAGE: true,
	SUMMARY: true,
	TAGS: true,
	GENRES: true,
	ISBN: true,
	AGE_RATING: true,
	PAGE_COUNT: true
};

export function isApplyableMetadataField(field: MetadataField): boolean {
	return APPLYABLE_METADATA_FIELDS[field] === true;
}

export const FIELD_MODES: IngestMetadataFieldMode[] = [
	'KEEP_EXISTING',
	'CANDIDATE',
	'MANUAL',
	'CLEAR'
];

export function offsetPagination(pageSize = 50): Pagination {
	return { offset: { page: 1, pageSize, zeroBased: false } };
}

export function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ['KiB', 'MiB', 'GiB'];
	let value = bytes / 1024;
	let unit = units[0];
	for (let index = 1; index < units.length && value >= 1024; index += 1) {
		value /= 1024;
		unit = units[index];
	}
	return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
}

export function formatDate(value: string | null | undefined): string {
	if (!value) return '—';
	const date = new Date(value);
	if (Number.isNaN(date.valueOf())) return value;
	return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
}

// `h:mm:ss`, spelled exactly like the `duration`/`start` strings the audio
// probe already returns, so a client-formatted offset and a server-formatted
// one line up in the same table. Hours are never zero-padded.
export function formatDurationMs(milliseconds: number): string {
	const seconds = Math.floor(Math.max(0, milliseconds) / 1000);
	const minutes = Math.floor(seconds / 60) % 60;
	return `${Math.floor(seconds / 3600)}:${String(minutes).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
}

export function humanize(value: string): string {
	return value
		.toLowerCase()
		.split('_')
		.map((part) => part.charAt(0).toUpperCase() + part.slice(1))
		.join(' ');
}

export function statusTone(status: IngestDropItemStatus | string): 'default' | 'secondary' | 'destructive' | 'outline' {
	if (status === 'FAILED' || status === 'REJECTED') return 'destructive';
	if (status === 'READY' || status === 'COMMITTED' || status === 'COMPLETED') return 'default';
	if (status === 'AWAITING_REVIEW' || status === 'PAUSED') return 'secondary';
	return 'outline';
}

export function progressPercent(completed: number, total: number): number {
	if (total <= 0) return 0;
	return Math.min(100, Math.max(0, Math.round((completed / total) * 100)));
}

export function parseJsonObject(value: unknown): Record<string, unknown> {
	if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
	return value as Record<string, unknown>;
}
