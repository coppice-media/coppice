import type {
	ConsoleBookRowFragment,
	FileStatus,
	LibraryPattern,
	LibraryType,
	MediaFilterInput,
	ReadingStatus
} from '$lib/graphql/generated/graphql';

export const LIBRARY_TYPE_LABELS: Record<LibraryType, string> = {
	BOOK: 'Books',
	COMIC: 'Comics',
	LIGHT_NOVEL: 'Light novels',
	MANGA: 'Manga',
	MANHWA: 'Manhwa',
	MIXED: 'Mixed',
	WEB_NOVEL: 'Web novels',
	WEBTOON: 'Webtoons'
};

export const LIBRARY_PATTERN_LABELS: Record<LibraryPattern, string> = {
	SERIES_BASED: 'One series per top-level folder',
	COLLECTION_BASED: 'Collections of nested series'
};

export const READING_STATUS_LABELS: Record<ReadingStatus, string> = {
	READING: 'Reading',
	FINISHED: 'Finished',
	ABANDONED: 'Abandoned',
	NOT_STARTED: 'Unread'
};

export const READING_STATUS_OPTIONS = (
	Object.keys(READING_STATUS_LABELS) as ReadingStatus[]
).map((value) => ({ value, label: READING_STATUS_LABELS[value] }));

/**
 * The book formats a Stump scanner persists — every extension
 * `ContentType::from_extension` maps to a supported, non-image type
 * (`crates/media/src/content_type.rs`).
 */
export const BOOK_FORMATS = ['cbz', 'cbr', 'zip', 'rar', 'epub', 'pdf'] as const;

export const FILE_STATUS_LABELS: Record<FileStatus, string> = {
	READY: 'Ready',
	UNKNOWN: 'Unknown',
	ERROR: 'Error',
	MISSING: 'Missing',
	UNSUPPORTED: 'Unsupported'
};

/** Sentinel for a select whose "no filter" entry cannot be an empty string. */
export const ANY_OPTION = '__any__';

/** The union member of `PaginationInfo` an offset page carries. */
export type OffsetPage = {
	totalItems: number;
	totalPages: number;
	currentPage: number;
	pageSize: number;
};

export function offsetPage(info: unknown): OffsetPage | null {
	if (!info || typeof info !== 'object' || !('totalItems' in info)) return null;
	return info as OffsetPage;
}

export type BookProgress = {
	status: ReadingStatus;
	/** Percentage of the active readthrough, `null` when nothing is tracked. */
	percentage: number | null;
	page: number | null;
};

/**
 * The reading state of a book, derived exactly like the server's
 * `ReadingStatus`: an active session means *reading*, a completed readthrough
 * means *finished* (or *abandoned* when it was a did-not-finish).
 */
export function bookProgress(book: ConsoleBookRowFragment): BookProgress {
	const latest = book.readHistory.at(-1);
	const percentage =
		book.readProgress?.percentageCompleted === null ||
		book.readProgress?.percentageCompleted === undefined
			? null
			: Number(book.readProgress.percentageCompleted);

	if (book.readProgress) {
		return {
			status: 'READING',
			percentage: Number.isFinite(percentage) ? percentage : null,
			page: book.readProgress.page ?? null
		};
	}
	if (latest) {
		return { status: latest.dnf ? 'ABANDONED' : 'FINISHED', percentage: null, page: null };
	}
	return { status: 'NOT_STARTED', percentage: null, page: null };
}

/** `0–100` for the progress bar of a book row. */
export function progressPercent(progress: BookProgress): number {
	if (progress.status === 'FINISHED') return 100;
	if (progress.percentage === null) return 0;
	// The server stores a fraction for epubs and a percentage for paged books.
	const value = progress.percentage <= 1 ? progress.percentage * 100 : progress.percentage;
	return Math.max(0, Math.min(100, Math.round(value)));
}

/** The facets the library screen filters books by, as URL-friendly values. */
export type BookFacets = {
	status: ReadingStatus | null;
	format: string | null;
	tag: string | null;
	publisher: string | null;
	author: string | null;
	search: string | null;
};

export const NO_FACETS: BookFacets = {
	status: null,
	format: null,
	tag: null,
	publisher: null,
	author: null,
	search: null
};

/** Builds the `media` filter for a library (or series) plus the active facets. */
export function bookFilter(
	scope: { libraryId?: string; seriesId?: string },
	facets: BookFacets
): MediaFilterInput {
	const filter: MediaFilterInput = {};
	if (scope.seriesId) filter.seriesId = { eq: scope.seriesId };
	else if (scope.libraryId) filter.series = { libraryId: { eq: scope.libraryId } };

	if (facets.status) filter.readingStatus = { is: facets.status };
	if (facets.format) filter.extension = { eq: facets.format };
	if (facets.tag) filter.tags = { eq: facets.tag };
	if (facets.search) filter.name = { contains: facets.search };

	const metadata: NonNullable<MediaFilterInput['metadata']> = {};
	if (facets.publisher) metadata.publisher = { eq: facets.publisher };
	// Writers are stored as one comma-separated column, so an author is a
	// substring match rather than an equality one.
	if (facets.author) metadata.writers = { contains: facets.author };
	if (Object.keys(metadata).length) filter.metadata = metadata;

	return filter;
}

export function activeFacets(facets: BookFacets): { key: keyof BookFacets; label: string }[] {
	const labels: Record<keyof BookFacets, string> = {
		status: 'Status',
		format: 'Format',
		tag: 'Tag',
		publisher: 'Publisher',
		author: 'Author',
		search: 'Search'
	};
	return (Object.keys(labels) as (keyof BookFacets)[])
		.filter((key) => facets[key])
		.map((key) => ({
			key,
			label: `${labels[key]}: ${key === 'status' ? READING_STATUS_LABELS[facets.status!] : facets[key]}`
		}));
}
