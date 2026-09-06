/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
/**
 * Exactly one of `drop_item_id` (staged item) or `media_id` (library-wide
 * rework target) must be provided.
 */
export type ApplyIngestMetadataInput = {
  dropItemId?: string | number | null | undefined;
  mediaId?: string | number | null | undefined;
  selections: Array<IngestMetadataFieldSelectionInput>;
  strategy?: MergeStrategy | null | undefined;
};

export type BulkApplyIngestMetadataInput = {
  dropItemIds: Array<string | number>;
  selections: Array<IngestMetadataFieldSelectionInput>;
  strategy?: MergeStrategy | null | undefined;
};

export type ComputedFilterLibraryType =
  {   is: LibraryType; isAnyOf?: never; isNoneOf?: never; isNot?: never; }
  |  { is?: never;   isAnyOf: Array<LibraryType>; isNoneOf?: never; isNot?: never; }
  |  { is?: never; isAnyOf?: never;   isNoneOf: Array<LibraryType>; isNot?: never; }
  |  { is?: never; isAnyOf?: never; isNoneOf?: never;   isNot: LibraryType; };

export type ComputedFilterReadingStatus =
  {   is: ReadingStatus; isAnyOf?: never; isNoneOf?: never; isNot?: never; }
  |  { is?: never;   isAnyOf: Array<ReadingStatus>; isNoneOf?: never; isNot?: never; }
  |  { is?: never; isAnyOf?: never;   isNoneOf: Array<ReadingStatus>; isNot?: never; }
  |  { is?: never; isAnyOf?: never; isNoneOf?: never;   isNot: ReadingStatus; };

/** Input object for creating a metadata provider configuration */
export type CreateMetadataProviderConfigInput = {
  /** The API token for authenticating with the provider */
  apiToken: string;
  /**
   * Optional expiration date for the API key. This is exclusively a QOL thing,
   * since the creds don't live within the management domain of Stump
   */
  apiTokenExpiresAt?: string | null | undefined;
  /** Auto-apply configuration */
  autoApplyConfig?: unknown;
  /** Whether the provider is enabled */
  enabled?: boolean | null | undefined;
  /** The provider type */
  providerType: MetadataProvider;
};

/** A simple cursor-based pagination input object */
export type CursorPagination = {
  after?: string | null | undefined;
  limit?: number;
};

/** A librarian decision about a recurring page hash inside a library */
export type DuplicatePageAction =
  /** The page was reviewed and stays visible; stop reporting it */
  | 'KEEP'
  /** Hide pages with this hash from every page-serving route */
  | 'SKIP';

export type EnqueueIngestAnalysisInput = {
  dropItemIds: Array<string | number>;
  force?: boolean;
};

export type FieldFilterFileStatus =
  {   anyOf: Array<FileStatus>; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never;   contains: FileStatus; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never;   endsWith: FileStatus; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never;   eq: FileStatus; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never;   excludes: FileStatus; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never;   like: FileStatus; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never;   likeAnyOf: Array<FileStatus>; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never;   likeNoneOf: Array<FileStatus>; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never;   neq: FileStatus; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never;   noneOf: Array<FileStatus>; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never;   startsWith: FileStatus; };

export type FieldFilterString =
  {   anyOf: Array<string>; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never;   contains: string; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never;   endsWith: string; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never;   eq: string; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never;   excludes: string; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never;   like: string; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never;   likeAnyOf: Array<string>; likeNoneOf?: never; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never;   likeNoneOf: Array<string>; neq?: never; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never;   neq: string; noneOf?: never; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never;   noneOf: Array<string>; startsWith?: never; }
  |  { anyOf?: never; contains?: never; endsWith?: never; eq?: never; excludes?: never; like?: never; likeAnyOf?: never; likeNoneOf?: never; neq?: never; noneOf?: never;   startsWith: string; };

/** The different statuses a file reference can have */
export type FileStatus =
  | 'ERROR'
  | 'MISSING'
  | 'READY'
  | 'UNKNOWN'
  | 'UNSUPPORTED';

export type IngestAnalysisPhase =
  | 'ANALYSIS'
  | 'COMMIT'
  | 'DONE'
  | 'IDENTIFY'
  | 'LOOKUP'
  | 'QUALITY'
  | 'REVIEW'
  | 'STAGING';

export type IngestCandidateStatus =
  | 'ACCEPTED'
  | 'PENDING'
  | 'REJECTED';

export type IngestDropItemStatus =
  | 'ANALYZING'
  | 'AWAITING_REVIEW'
  | 'COMMITTED'
  | 'FAILED'
  | 'READY'
  | 'RECEIVED'
  | 'REJECTED'
  | 'STAGED';

export type IngestMediaKind =
  | 'COMIC_ARCHIVE'
  | 'COMIC_RAR_ARCHIVE'
  | 'EPUB'
  | 'PDF'
  | 'UNKNOWN';

export type IngestMetadataFieldMode =
  | 'CANDIDATE'
  | 'CLEAR'
  | 'KEEP_EXISTING'
  | 'MANUAL';

export type IngestMetadataFieldSelectionInput = {
  candidateId?: string | number | null | undefined;
  field: MetadataField;
  mode: IngestMetadataFieldMode;
  value?: unknown;
};

export type IngestProviderCapability =
  | 'AI_ENRICHMENT'
  | 'IDENTIFY'
  | 'LOOKUP'
  | 'SEARCH'
  | 'TAGS';

export type IngestQualityStatus =
  | 'FAIL'
  | 'NOT_APPLICABLE'
  | 'PASS'
  | 'WARN';

export type IngestSettingValueType =
  | 'BOOLEAN'
  | 'INTEGER'
  | 'JSON'
  | 'NUMBER'
  | 'STRING';

export type IngestUploadFileInput = {
  file: File;
  relativePath?: string | null | undefined;
};

export type JobStatus =
  | 'CANCELLED'
  | 'COMPLETED'
  | 'FAILED'
  | 'PAUSED'
  | 'QUEUED'
  | 'RUNNING';

export type LibraryFilterInput = {
  _and?: Array<LibraryFilterInput> | null | undefined;
  _not?: Array<LibraryFilterInput> | null | undefined;
  _or?: Array<LibraryFilterInput> | null | undefined;
  id?: FieldFilterString | null | undefined;
  name?: FieldFilterString | null | undefined;
  path?: FieldFilterString | null | undefined;
};

/** The type of content a library contains */
export type LibraryType =
  | 'BOOK'
  | 'COMIC'
  | 'LIGHT_NOVEL'
  | 'MANGA'
  | 'MANHWA'
  | 'MIXED'
  | 'WEBTOON'
  | 'WEB_NOVEL';

export type MediaFilterInput = {
  _and?: Array<MediaFilterInput> | null | undefined;
  _not?: Array<MediaFilterInput> | null | undefined;
  _or?: Array<MediaFilterInput> | null | undefined;
  createdAt?: NumericFilterDateTime | null | undefined;
  extension?: FieldFilterString | null | undefined;
  id?: FieldFilterString | null | undefined;
  metadata?: MediaMetadataFilterInput | null | undefined;
  name?: FieldFilterString | null | undefined;
  pages?: NumericFilterI32 | null | undefined;
  path?: FieldFilterString | null | undefined;
  readingStatus?: ComputedFilterReadingStatus | null | undefined;
  series?: SeriesFilterInput | null | undefined;
  seriesId?: FieldFilterString | null | undefined;
  size?: NumericFilterI64 | null | undefined;
  status?: FieldFilterFileStatus | null | undefined;
  tags?: FieldFilterString | null | undefined;
  updatedAt?: NumericFilterDateTime | null | undefined;
};

export type MediaMetadataFilterInput = {
  _and?: Array<MediaMetadataFilterInput> | null | undefined;
  _not?: Array<MediaMetadataFilterInput> | null | undefined;
  _or?: Array<MediaMetadataFilterInput> | null | undefined;
  ageRating?: NumericFilterI32 | null | undefined;
  characters?: FieldFilterString | null | undefined;
  colorists?: FieldFilterString | null | undefined;
  coverArtists?: FieldFilterString | null | undefined;
  day?: NumericFilterI32 | null | undefined;
  editors?: FieldFilterString | null | undefined;
  genres?: FieldFilterString | null | undefined;
  inkers?: FieldFilterString | null | undefined;
  letterers?: FieldFilterString | null | undefined;
  links?: FieldFilterString | null | undefined;
  month?: NumericFilterI32 | null | undefined;
  pencillers?: FieldFilterString | null | undefined;
  publisher?: FieldFilterString | null | undefined;
  series?: FieldFilterString | null | undefined;
  summary?: FieldFilterString | null | undefined;
  teams?: FieldFilterString | null | undefined;
  title?: FieldFilterString | null | undefined;
  writers?: FieldFilterString | null | undefined;
  year?: NumericFilterI32 | null | undefined;
};

/** How to merge external metadata values onto existing entity metadata */
export type MergeStrategy =
  /** FillGaps and merge/dedupe for array fields */
  | 'FILL_AND_MERGE_LISTS'
  /** Only populate fields that are currently nullish */
  | 'FILL_GAPS'
  /** Overwrite existing values with (truthy) external data */
  | 'PREFER_EXTERNAL'
  /** PreferExternal for scalars, merge/dedupe for array fields */
  | 'PREFER_EXTERNAL_AND_MERGE_LISTS';

/**
 * Represents a specific metadata field that can be locked or configured
 * for per-field merge strategies
 */
export type MetadataField =
  | 'AGE_RATING'
  | 'ARTISTS'
  | 'BOOK_TYPE'
  | 'CHARACTERS'
  | 'COLORISTS'
  | 'COMIC_ID'
  | 'COMIC_IMAGE'
  | 'COVER'
  | 'COVER_ARTISTS'
  | 'DESCRIPTION_FORMATTED'
  | 'EDITORS'
  | 'FORMAT'
  | 'GENRES'
  | 'IDENTIFIER_AMAZON'
  | 'IDENTIFIER_CALIBRE'
  | 'IDENTIFIER_GOOGLE'
  | 'IDENTIFIER_MOBI_ASIN'
  | 'IDENTIFIER_UUID'
  | 'IMPRINT'
  | 'INKERS'
  | 'ISBN'
  | 'LANGUAGE'
  | 'LETTERERS'
  | 'LINKS'
  | 'META_TYPE'
  | 'NOTES'
  | 'NUMBER'
  | 'PAGE_COUNT'
  | 'PENCILLERS'
  | 'PUBLICATION_RUN'
  | 'PUBLISHER'
  | 'RELEASE_DATE'
  | 'SERIES'
  | 'SERIES_GROUP'
  | 'STATUS'
  | 'STORY_ARC'
  | 'STORY_ARC_NUMBER'
  | 'SUMMARY'
  | 'TAGS'
  | 'TEAMS'
  | 'TITLE'
  | 'TITLE_SORT'
  | 'VOLUME_COUNT'
  | 'WRITERS'
  | 'YEAR';

/** The supported external metadata providers */
export type MetadataProvider =
  /** AniList (https://anilist.co) */
  | 'ANI_LIST'
  /** ComicVine (https://comicvine.gamespot.com/api/) */
  | 'COMIC_VINE'
  /** Google Books (https://www.googleapis.com/books/v1) */
  | 'GOOGLE_BOOKS'
  /** Hardcover (https://hardcover.app) */
  | 'HARDCOVER'
  /** MyAnimeList (https://myanimelist.net/apiconfig/references/api/v2) */
  | 'MAL'
  /** MangaDex (https://api.mangadex.org) */
  | 'MANGA_DEX'
  /** MangaUpdates (https://api.mangaupdates.com/v1) */
  | 'MANGA_UPDATES'
  /** Metron (https://metron.cloud/api/) */
  | 'METRON'
  /** Open Library (https://openlibrary.org) */
  | 'OPEN_LIBRARY';

export type NumericFilterDateTime =
  {   anyOf: Array<string>; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never;   eq: string; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never;   gt: string; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never;   gte: string; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never;   lt: string; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never;   lte: string; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never;   neq: string; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never;   noneOf: Array<string>; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never;   range: NumericRangeDateTime; };

export type NumericFilterI32 =
  {   anyOf: Array<number>; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never;   eq: number; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never;   gt: number; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never;   gte: number; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never;   lt: number; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never;   lte: number; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never;   neq: number; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never;   noneOf: Array<number>; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never;   range: NumericRangeI32; };

export type NumericFilterI64 =
  {   anyOf: Array<number>; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never;   eq: number; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never;   gt: number; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never;   gte: number; lt?: never; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never;   lt: number; lte?: never; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never;   lte: number; neq?: never; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never;   neq: number; noneOf?: never; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never;   noneOf: Array<number>; range?: never; }
  |  { anyOf?: never; eq?: never; gt?: never; gte?: never; lt?: never; lte?: never; neq?: never; noneOf?: never;   range: NumericRangeI64; };

export type NumericRangeDateTime = {
  from: string;
  inclusive: boolean;
  to: string;
};

export type NumericRangeI32 = {
  from: number;
  inclusive: boolean;
  to: number;
};

export type NumericRangeI64 = {
  from: number;
  inclusive: boolean;
  to: number;
};

/** A simple offset-based pagination input object */
export type OffsetPagination = {
  /**
   * The page to start from. This is 1-based by default, but can be
   * changed to 0-based by setting the `zero_based` field to true.
   */
  page: number;
  /** The number of items to return per page. This is 20 by default. */
  pageSize?: number | null | undefined;
  /** Whether or not the page is zero-based. This is false by default. */
  zeroBased?: boolean | null | undefined;
};

/**
 * A union of the supported pagination flavors which Stump supports. The resulting
 * response will be dependent on the pagination type used, e.g. a [CursorPaginatedResponse]
 * will be returned if the [CursorPagination] type is used.
 *
 * You may use a conditional fragment in your GraphQL query for type-specific fields:
 * ```graphql
 * query MyQuery {
 * media(pagination: { offset: { page: 1, pageSize: 20 } }) {
 * ... on OffsetPaginationInfo {
 * totalPages
 * currentPage
 * }
 * }
 * }
 * ```
 *
 * A special case is the `None` variant, which will return an offset-based pagination info
 * object based on the size of the result set. This will not paginate the results, so be
 * cautious when using this with large result sets.
 *
 * **Note**: Be sure to call [Pagination::resolve] before using the pagination object
 * to ensure that the pagination object is in a valid state.
 */
export type Pagination =
  {   cursor: CursorPagination; none?: never; offset?: never; }
  |  { cursor?: never;   none: Unpaginated; offset?: never; }
  |  { cursor?: never; none?: never;   offset: OffsetPagination; };

/** A patch equivalent of [CreateMetadataProviderConfigInput], i.e. just with optional fields. */
export type PatchMetadataProviderConfigInput = {
  /** The API token for authenticating with the provider */
  apiToken?: string | null | undefined;
  /**
   * Optional expiration date for the API key. This is exclusively a QOL thing,
   * since the creds don't live within the management domain of Stump
   */
  apiTokenExpiresAt?: string | null | undefined;
  /** Auto-apply configuration */
  autoApplyConfig?: unknown;
  /** Whether the provider is enabled */
  enabled?: boolean | null | undefined;
};

/**
 * the different reading statuses a book can be categorized as based on a user's
 * reading sessions
 */
export type ReadingStatus =
  /** a user actively started reading a book but decided not to finish it (i.e., dnf-ing a book) */
  | 'ABANDONED'
  /** there is at least one completed readthrough for this book */
  | 'FINISHED'
  /** no sessions have been recorded for this book */
  | 'NOT_STARTED'
  /**
   * there is an active reading session for this book. it may or may not have been completed in
   * the past, this is strictly about the presence of an active session
   */
  | 'READING';

export type SeriesFilterInput = {
  _and?: Array<SeriesFilterInput> | null | undefined;
  _not?: Array<SeriesFilterInput> | null | undefined;
  _or?: Array<SeriesFilterInput> | null | undefined;
  library?: LibraryFilterInput | null | undefined;
  libraryId?: FieldFilterString | null | undefined;
  libraryType?: ComputedFilterLibraryType | null | undefined;
  metadata?: SeriesMetadataFilterInput | null | undefined;
  name?: FieldFilterString | null | undefined;
  path?: FieldFilterString | null | undefined;
  readingStatus?: ComputedFilterReadingStatus | null | undefined;
};

export type SeriesMetadataFilterInput = {
  _and?: Array<SeriesMetadataFilterInput> | null | undefined;
  _not?: Array<SeriesMetadataFilterInput> | null | undefined;
  _or?: Array<SeriesMetadataFilterInput> | null | undefined;
  ageRating?: NumericFilterI32 | null | undefined;
  booktype?: FieldFilterString | null | undefined;
  comicid?: NumericFilterI32 | null | undefined;
  imprint?: FieldFilterString | null | undefined;
  metaType?: FieldFilterString | null | undefined;
  publisher?: FieldFilterString | null | undefined;
  status?: FieldFilterString | null | undefined;
  summary?: FieldFilterString | null | undefined;
  title?: FieldFilterString | null | undefined;
  volume?: NumericFilterI32 | null | undefined;
  year?: NumericFilterI32 | null | undefined;
};

export type SetIngestProviderSettingsInput = {
  enabled?: boolean | null | undefined;
  optedIn?: boolean | null | undefined;
  providerId: string;
  settings?: unknown;
};

export type SetIngestQualityCheckSettingsInput = {
  checkId: string;
  enabled?: boolean | null | undefined;
  settings?: unknown;
};

export type StageIngestUploadsInput = {
  files: Array<IngestUploadFileInput>;
  idempotencyKey?: string | null | undefined;
  libraryId: string | number;
  startAnalysis?: boolean;
};

/**
 * A simple pagination input object which does not paginate. An explicit struct is
 * required as a limitation of async_graphql's [OneofObject], which doesn't allow
 * for empty variants.
 */
export type Unpaginated = {
  unpaginated: boolean;
};

export type DuplicatePageCandidatesQueryVariables = Exact<{
  libraryId: string | number;
  minBooks?: number | null | undefined;
  limit?: number | null | undefined;
}>;


export type DuplicatePageCandidatesQuery = { duplicatePageCandidates: Array<{ dhash: string, bookCount: number, pageCount: number, occurrences: Array<{ mediaId: string, mediaName: string, page: number, visiblePage: number | null, dhash: string }> }> };

export type KnownDuplicatePagesQueryVariables = Exact<{
  libraryId: string | number;
}>;


export type KnownDuplicatePagesQuery = { knownDuplicatePages: Array<{ libraryId: string, dhash: string, action: DuplicatePageAction, createdBy: string | null, createdAt: string }> };

export type MarkDuplicatePageMutationVariables = Exact<{
  libraryId: string | number;
  dhash: string;
  action: DuplicatePageAction;
}>;


export type MarkDuplicatePageMutation = { markDuplicatePage: { libraryId: string, dhash: string, action: DuplicatePageAction, createdAt: string } };

export type UnmarkDuplicatePageMutationVariables = Exact<{
  libraryId: string | number;
  dhash: string;
}>;


export type UnmarkDuplicatePageMutation = { unmarkDuplicatePage: boolean };

export type LibraryMediaQueryVariables = Exact<{
  filter: MediaFilterInput;
  pagination: Pagination;
}>;


export type LibraryMediaQuery = { media: { nodes: Array<{ id: string, name: string, resolvedName: string, path: string, pages: number, libraryId: string, thumbnail: { url: string }, series: { id: string, name: string } }>, pageInfo:
      | { currentCursor: string | null, nextCursor: string | null, limit: number }
      | { totalPages: number, totalItems: number, currentPage: number, pageSize: number }
     } };

export type IngestMediaQualityScoreQueryVariables = Exact<{
  mediaId: string | number;
}>;


export type IngestMediaQualityScoreQuery = { ingestMediaQualityReport: { id: string, score: number } | null };

export type IngestMediaQualityReportQueryVariables = Exact<{
  mediaId: string | number;
}>;


export type IngestMediaQualityReportQuery = { ingestMediaQualityReport: { id: string, score: number, algorithmVersion: string, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, contribution: number, normalizedScore: number, evidence: unknown }> } | null };

export type IngestMediaMetadataCandidatesQueryVariables = Exact<{
  mediaId: string | number;
}>;


export type IngestMediaMetadataCandidatesQuery = { ingestMediaMetadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }> };

export type LibraryAnalysisJobQueryVariables = Exact<{
  id: string | number;
}>;


export type LibraryAnalysisJobQuery = { ingestAnalysisJob: { id: string, status: JobStatus, phase: IngestAnalysisPhase, error: string | null } | null };

export type RunLibraryQualityMutationVariables = Exact<{
  mediaIds: Array<string | number> | string | number;
}>;


export type RunLibraryQualityMutation = { runLibraryQuality: { id: string, status: JobStatus, phase: IngestAnalysisPhase } };

export type MatchLibraryMediaMutationVariables = Exact<{
  mediaIds: Array<string | number> | string | number;
  providers?: Array<string> | string | null | undefined;
}>;


export type MatchLibraryMediaMutation = { matchLibraryMedia: { id: string, status: JobStatus, phase: IngestAnalysisPhase } };

export type IngestDropItemFieldsFragment = { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null };

type IngestPageInfoFields_CursorPaginationInfo_Fragment = { currentCursor: string | null, nextCursor: string | null, limit: number };

type IngestPageInfoFields_OffsetPaginationInfo_Fragment = { totalPages: number, totalItems: number, currentPage: number, pageSize: number };

export type IngestPageInfoFieldsFragment =
  | IngestPageInfoFields_CursorPaginationInfo_Fragment
  | IngestPageInfoFields_OffsetPaginationInfo_Fragment
;

export type LibrariesQueryVariables = Exact<{
  pagination: Pagination;
}>;


export type LibrariesQuery = { libraries: { nodes: Array<{ id: string, name: string, emoji: string | null }>, pageInfo:
      | { currentCursor: string | null, nextCursor: string | null, limit: number }
      | { totalPages: number, totalItems: number, currentPage: number, pageSize: number }
     } };

export type IngestDropFolderAndItemsQueryVariables = Exact<{
  libraryId: string | number;
  status?: IngestDropItemStatus | null | undefined;
  pagination: Pagination;
}>;


export type IngestDropFolderAndItemsQuery = { ingestDropFolder: { libraryId: string, displayPath: string, enabled: boolean, pendingCount: number, lastDiscoveredAt: string | null } | null, ingestDropItems: { nodes: Array<{ id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null }>, pageInfo:
      | { currentCursor: string | null, nextCursor: string | null, limit: number }
      | { totalPages: number, totalItems: number, currentPage: number, pageSize: number }
     } };

export type IngestItemQueryVariables = Exact<{
  id: string | number;
}>;


export type IngestItemQuery = { ingestItem: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null } | null };

export type IngestAnalysisQueueQueryVariables = Exact<{
  status?: JobStatus | null | undefined;
  pagination: Pagination;
}>;


export type IngestAnalysisQueueQuery = { ingestAnalysisQueue: { nodes: Array<{ id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null }>, pageInfo:
      | { currentCursor: string | null, nextCursor: string | null, limit: number }
      | { totalPages: number, totalItems: number, currentPage: number, pageSize: number }
     } };

export type IngestAnalysisJobQueryVariables = Exact<{
  id: string | number;
}>;


export type IngestAnalysisJobQuery = { ingestAnalysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null };

export type IngestReworkItemsQueryVariables = Exact<{
  minScore?: number | null | undefined;
  pagination: Pagination;
}>;


export type IngestReworkItemsQuery = { ingestReworkItems: { nodes: Array<{ item: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null }, reasons: Array<{ checkId: string, status: IngestQualityStatus, message: string }> }>, pageInfo:
      | { currentCursor: string | null, nextCursor: string | null, limit: number }
      | { totalPages: number, totalItems: number, currentPage: number, pageSize: number }
     } };

export type IngestBulkItemsQueryVariables = Exact<{
  ids: Array<string | number> | string | number;
}>;


export type IngestBulkItemsQuery = { ingestBulkItems: Array<{ id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null }> };

export type IngestProviderCatalogQueryVariables = Exact<{
  includeDisabled: boolean;
}>;


export type IngestProviderCatalogQuery = { ingestProviderCatalog: Array<{ id: string, name: string, version: string, configured: boolean, capabilities: Array<IngestProviderCapability>, supportedMediaTypes: Array<string>, enabledByDefault: boolean, requiresApiToken: boolean, helpUrl: string | null, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null, helpUrl: string | null }> }> };

export type IngestProviderSettingsQueryVariables = Exact<{
  providerId: string;
}>;


export type IngestProviderSettingsQuery = { ingestProviderSettings: { enabled: boolean, optedIn: boolean, updatedAt: string | null, provider: { id: string, name: string, version: string, configured: boolean, capabilities: Array<IngestProviderCapability>, supportedMediaTypes: Array<string>, enabledByDefault: boolean, requiresApiToken: boolean, helpUrl: string | null, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null, helpUrl: string | null }> }, settings: Array<{ key: string, configured: boolean, secret: boolean, value: unknown }> } | null };

export type IngestQualityCheckCatalogQueryVariables = Exact<{
  includeDisabled: boolean;
}>;


export type IngestQualityCheckCatalogQuery = { ingestQualityCheckCatalog: Array<{ id: string, name: string, version: string, available: boolean, weight: number, enabled: boolean, supportedMediaTypes: Array<string>, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null }> }> };

export type IngestQualityCheckSettingsQueryVariables = Exact<{
  checkId: string;
}>;


export type IngestQualityCheckSettingsQuery = { ingestQualityCheckSettings: { checkId: string, enabled: boolean, updatedAt: string | null, settings: Array<{ key: string, configured: boolean, secret: boolean, value: unknown }> } | null };

export type StageIngestUploadsMutationVariables = Exact<{
  input: StageIngestUploadsInput;
}>;


export type StageIngestUploadsMutation = { stageIngestUploads: { deduplicated: number, items: Array<{ id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null }> } };

export type ScanIngestDropFolderMutationVariables = Exact<{
  libraryId: string | number;
}>;


export type ScanIngestDropFolderMutation = { scanIngestDropFolder: { libraryId: string, displayPath: string, enabled: boolean, pendingCount: number, lastDiscoveredAt: string | null } };

export type EnqueueIngestAnalysisMutationVariables = Exact<{
  input: EnqueueIngestAnalysisInput;
}>;


export type EnqueueIngestAnalysisMutation = { enqueueIngestAnalysis: Array<{ id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null }> };

export type RequeueIngestAnalysisMutationVariables = Exact<{
  dropItemId: string | number;
  force: boolean;
}>;


export type RequeueIngestAnalysisMutation = { requeueIngestAnalysis: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } };

export type PauseIngestAnalysisMutationVariables = Exact<{
  jobId: string | number;
}>;


export type PauseIngestAnalysisMutation = { pauseIngestAnalysis: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } };

export type ResumeIngestAnalysisMutationVariables = Exact<{
  jobId: string | number;
}>;


export type ResumeIngestAnalysisMutation = { resumeIngestAnalysis: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } };

export type RetryIngestAnalysisMutationVariables = Exact<{
  jobId: string | number;
}>;


export type RetryIngestAnalysisMutation = { retryIngestAnalysis: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } };

export type CancelIngestAnalysisMutationVariables = Exact<{
  jobId: string | number;
}>;


export type CancelIngestAnalysisMutation = { cancelIngestAnalysis: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } };

export type DiscardIngestItemMutationVariables = Exact<{
  dropItemId: string | number;
  reason?: string | null | undefined;
}>;


export type DiscardIngestItemMutation = { discardIngestItem: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null } };

export type ApplyIngestMetadataMutationVariables = Exact<{
  input: ApplyIngestMetadataInput;
}>;


export type ApplyIngestMetadataMutation = { applyIngestMetadata: { dropItem: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null } | null, media: { id: string, name: string, resolvedName: string } | null } };

export type BulkApplyIngestMetadataMutationVariables = Exact<{
  input: BulkApplyIngestMetadataInput;
}>;


export type BulkApplyIngestMetadataMutation = { bulkApplyIngestMetadata: { applied: Array<{ id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null }>, failures: Array<{ dropItemId: string, message: string }> } };

export type ApproveIngestItemMutationVariables = Exact<{
  dropItemId: string | number;
  strategy?: MergeStrategy | null | undefined;
}>;


export type ApproveIngestItemMutation = { approveIngestItem: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null } };

export type RejectIngestItemMutationVariables = Exact<{
  dropItemId: string | number;
  reason?: string | null | undefined;
}>;


export type RejectIngestItemMutation = { rejectIngestItem: { id: string, libraryId: string, createdBy: string, filename: string, relativePath: string | null, sizeBytes: number, sourceSha256: string, mediaType: string, status: IngestDropItemStatus, revision: number, pendingFields: unknown, error: string | null, createdAt: string, updatedAt: string, analysisJob: { id: string, dropItemId: string | null, jobId: string | null, status: JobStatus, phase: IngestAnalysisPhase, priorityScore: number, attempt: number, queuedAt: string, startedAt: string | null, completedAt: string | null, error: string | null } | null, qualityReport: { id: string, dropItemId: string | null, sourceSha256: string, algorithmVersion: string, score: number, generatedAt: string, checks: Array<{ checkId: string, label: string, status: IngestQualityStatus, weight: number, normalizedScore: number, contribution: number, evidence: unknown }> } | null, metadataCandidates: Array<{ id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string }>, media: { id: string, name: string } | null, series: { id: string, name: string } | null } };

export type SetIngestProviderSettingsMutationVariables = Exact<{
  input: SetIngestProviderSettingsInput;
}>;


export type SetIngestProviderSettingsMutation = { setIngestProviderSettings: { enabled: boolean, optedIn: boolean, updatedAt: string | null, provider: { id: string, name: string, version: string, configured: boolean, capabilities: Array<IngestProviderCapability>, supportedMediaTypes: Array<string>, enabledByDefault: boolean, requiresApiToken: boolean, helpUrl: string | null, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null, helpUrl: string | null }> }, settings: Array<{ key: string, configured: boolean, secret: boolean, value: unknown }> } };

export type VerifyIngestProviderMutationVariables = Exact<{
  providerId: string;
  settings?: unknown;
}>;


export type VerifyIngestProviderMutation = { verifyIngestProvider: { responseStatus: number, isValid: boolean, error: string | null } };

export type MetadataProviderConfigsQueryVariables = Exact<{ [key: string]: never; }>;


export type MetadataProviderConfigsQuery = { metadataProviderConfigs: Array<{ id: number, providerType: MetadataProvider, enabled: boolean, createdAt: string, updatedAt: string | null }> };

export type CreateMetadataProviderMutationVariables = Exact<{
  input: CreateMetadataProviderConfigInput;
}>;


export type CreateMetadataProviderMutation = { createMetadataProvider: { id: number, providerType: MetadataProvider, enabled: boolean } };

export type UpdateMetadataProviderMutationVariables = Exact<{
  id: number;
  input: PatchMetadataProviderConfigInput;
}>;


export type UpdateMetadataProviderMutation = { updateMetadataProvider: { id: number, providerType: MetadataProvider, enabled: boolean } };

export type DeleteMetadataProviderMutationVariables = Exact<{
  id: number;
}>;


export type DeleteMetadataProviderMutation = { deleteMetadataProvider: { id: number } };

export type SetIngestQualityCheckSettingsMutationVariables = Exact<{
  input: SetIngestQualityCheckSettingsInput;
}>;


export type SetIngestQualityCheckSettingsMutation = { setIngestQualityCheckSettings: { checkId: string, enabled: boolean, updatedAt: string | null, settings: Array<{ key: string, configured: boolean, secret: boolean, value: unknown }> } };

export type IngestProgressSubscriptionVariables = Exact<{
  libraryId?: string | number | null | undefined;
  dropItemId?: string | number | null | undefined;
  analysisJobId?: string | number | null | undefined;
  afterEventId?: string | number | null | undefined;
}>;


export type IngestProgressSubscription = { ingestProgress: { eventId: string, emittedAt: string, libraryId: string, dropItemId: string, analysisJobId: string | null, status: IngestDropItemStatus, phase: IngestAnalysisPhase, completed: number, total: number, score: number | null, message: string | null } };

export type IngestProviderSearchQueryVariables = Exact<{
  query: string;
  mediaKind?: IngestMediaKind | null | undefined;
  providers?: Array<string> | string | null | undefined;
  limit?: number | null | undefined;
}>;


export type IngestProviderSearchQuery = { ingestProviderSearch: Array<{ providerId: string, externalId: string, title: string, year: number | null, coverUrl: string | null, summary: string | null, score: number }> };

export type LookupIngestCandidateMutationVariables = Exact<{
  dropItemId: string | number;
  providerId: string;
  externalId: string;
}>;


export type LookupIngestCandidateMutation = { lookupIngestCandidate: { id: string, dropItemId: string | null, provider: string, providerVersion: string, model: string | null, confidence: number, fields: unknown, fieldConfidences: unknown, sourceSha256: string, provenance: unknown, status: IngestCandidateStatus, createdAt: string } };

export const IngestDropItemFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<IngestDropItemFieldsFragment, unknown>;
export const IngestPageInfoFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<IngestPageInfoFieldsFragment, unknown>;
export const DuplicatePageCandidatesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"DuplicatePageCandidates"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"minBooks"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"limit"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"duplicatePageCandidates"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"minBooks"},"value":{"kind":"Variable","name":{"kind":"Name","value":"minBooks"}}},{"kind":"Argument","name":{"kind":"Name","value":"limit"},"value":{"kind":"Variable","name":{"kind":"Name","value":"limit"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"dhash"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"pageCount"}},{"kind":"Field","name":{"kind":"Name","value":"occurrences"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"mediaId"}},{"kind":"Field","name":{"kind":"Name","value":"mediaName"}},{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"visiblePage"}},{"kind":"Field","name":{"kind":"Name","value":"dhash"}}]}}]}}]}}]} as unknown as DocumentNode<DuplicatePageCandidatesQuery, DuplicatePageCandidatesQueryVariables>;
export const KnownDuplicatePagesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"KnownDuplicatePages"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"knownDuplicatePages"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"dhash"}},{"kind":"Field","name":{"kind":"Name","value":"action"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}}]}}]} as unknown as DocumentNode<KnownDuplicatePagesQuery, KnownDuplicatePagesQueryVariables>;
export const MarkDuplicatePageDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"MarkDuplicatePage"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dhash"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"action"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"DuplicatePageAction"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"markDuplicatePage"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"dhash"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dhash"}}},{"kind":"Argument","name":{"kind":"Name","value":"action"},"value":{"kind":"Variable","name":{"kind":"Name","value":"action"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"dhash"}},{"kind":"Field","name":{"kind":"Name","value":"action"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}}]}}]} as unknown as DocumentNode<MarkDuplicatePageMutation, MarkDuplicatePageMutationVariables>;
export const UnmarkDuplicatePageDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"UnmarkDuplicatePage"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dhash"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"unmarkDuplicatePage"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"dhash"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dhash"}}}]}]}}]} as unknown as DocumentNode<UnmarkDuplicatePageMutation, UnmarkDuplicatePageMutationVariables>;
export const LibraryMediaDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"LibraryMedia"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"MediaFilterInput"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"media"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestPageInfoFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<LibraryMediaQuery, LibraryMediaQueryVariables>;
export const IngestMediaQualityScoreDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestMediaQualityScore"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestMediaQualityReport"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"score"}}]}}]}}]} as unknown as DocumentNode<IngestMediaQualityScoreQuery, IngestMediaQualityScoreQueryVariables>;
export const IngestMediaQualityReportDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestMediaQualityReport"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestMediaQualityReport"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}}]}}]} as unknown as DocumentNode<IngestMediaQualityReportQuery, IngestMediaQualityReportQueryVariables>;
export const IngestMediaMetadataCandidatesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestMediaMetadataCandidates"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestMediaMetadataCandidates"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}}]}}]} as unknown as DocumentNode<IngestMediaMetadataCandidatesQuery, IngestMediaMetadataCandidatesQueryVariables>;
export const LibraryAnalysisJobDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"LibraryAnalysisJob"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestAnalysisJob"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<LibraryAnalysisJobQuery, LibraryAnalysisJobQueryVariables>;
export const RunLibraryQualityDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RunLibraryQuality"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"runLibraryQuality"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaIds"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}}]}}]}}]} as unknown as DocumentNode<RunLibraryQualityMutation, RunLibraryQualityMutationVariables>;
export const MatchLibraryMediaDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"MatchLibraryMedia"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"providers"}},"type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"matchLibraryMedia"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaIds"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}}},{"kind":"Argument","name":{"kind":"Name","value":"providers"},"value":{"kind":"Variable","name":{"kind":"Name","value":"providers"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}}]}}]}}]} as unknown as DocumentNode<MatchLibraryMediaMutation, MatchLibraryMediaMutationVariables>;
export const LibrariesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"Libraries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestPageInfoFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<LibrariesQuery, LibrariesQueryVariables>;
export const IngestDropFolderAndItemsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestDropFolderAndItems"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"status"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItemStatus"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestDropFolder"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"displayPath"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"pendingCount"}},{"kind":"Field","name":{"kind":"Name","value":"lastDiscoveredAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"ingestDropItems"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"status"},"value":{"kind":"Variable","name":{"kind":"Name","value":"status"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestPageInfoFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<IngestDropFolderAndItemsQuery, IngestDropFolderAndItemsQueryVariables>;
export const IngestItemDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestItem"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestItem"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<IngestItemQuery, IngestItemQueryVariables>;
export const IngestAnalysisQueueDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestAnalysisQueue"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"status"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"JobStatus"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestAnalysisQueue"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"status"},"value":{"kind":"Variable","name":{"kind":"Name","value":"status"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestPageInfoFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<IngestAnalysisQueueQuery, IngestAnalysisQueueQueryVariables>;
export const IngestAnalysisJobDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestAnalysisJob"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestAnalysisJob"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<IngestAnalysisJobQuery, IngestAnalysisJobQueryVariables>;
export const IngestReworkItemsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestReworkItems"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"minScore"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"Float"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestReworkItems"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"minScore"},"value":{"kind":"Variable","name":{"kind":"Name","value":"minScore"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"item"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"reasons"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"message"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestPageInfoFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestPageInfoFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"CursorPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"currentCursor"}},{"kind":"Field","name":{"kind":"Name","value":"nextCursor"}},{"kind":"Field","name":{"kind":"Name","value":"limit"}}]}}]}}]} as unknown as DocumentNode<IngestReworkItemsQuery, IngestReworkItemsQueryVariables>;
export const IngestBulkItemsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestBulkItems"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"ids"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestBulkItems"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"ids"},"value":{"kind":"Variable","name":{"kind":"Name","value":"ids"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<IngestBulkItemsQuery, IngestBulkItemsQueryVariables>;
export const IngestProviderCatalogDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestProviderCatalog"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"includeDisabled"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Boolean"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestProviderCatalog"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"includeDisabled"},"value":{"kind":"Variable","name":{"kind":"Name","value":"includeDisabled"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"version"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"capabilities"}},{"kind":"Field","name":{"kind":"Name","value":"supportedMediaTypes"}},{"kind":"Field","name":{"kind":"Name","value":"enabledByDefault"}},{"kind":"Field","name":{"kind":"Name","value":"requiresApiToken"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}}]}}]}}]}}]} as unknown as DocumentNode<IngestProviderCatalogQuery, IngestProviderCatalogQueryVariables>;
export const IngestProviderSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestProviderSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestProviderSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"providerId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"provider"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"version"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"capabilities"}},{"kind":"Field","name":{"kind":"Name","value":"supportedMediaTypes"}},{"kind":"Field","name":{"kind":"Name","value":"enabledByDefault"}},{"kind":"Field","name":{"kind":"Name","value":"requiresApiToken"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"optedIn"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"value"}}]}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<IngestProviderSettingsQuery, IngestProviderSettingsQueryVariables>;
export const IngestQualityCheckCatalogDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestQualityCheckCatalog"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"includeDisabled"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Boolean"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestQualityCheckCatalog"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"includeDisabled"},"value":{"kind":"Variable","name":{"kind":"Name","value":"includeDisabled"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"version"}},{"kind":"Field","name":{"kind":"Name","value":"available"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"supportedMediaTypes"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}}]}}]}}]}}]} as unknown as DocumentNode<IngestQualityCheckCatalogQuery, IngestQualityCheckCatalogQueryVariables>;
export const IngestQualityCheckSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestQualityCheckSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"checkId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestQualityCheckSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"checkId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"checkId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"value"}}]}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<IngestQualityCheckSettingsQuery, IngestQualityCheckSettingsQueryVariables>;
export const StageIngestUploadsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"StageIngestUploads"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"StageIngestUploadsInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"stageIngestUploads"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"items"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"deduplicated"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<StageIngestUploadsMutation, StageIngestUploadsMutationVariables>;
export const ScanIngestDropFolderDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ScanIngestDropFolder"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"scanIngestDropFolder"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"displayPath"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"pendingCount"}},{"kind":"Field","name":{"kind":"Name","value":"lastDiscoveredAt"}}]}}]}}]} as unknown as DocumentNode<ScanIngestDropFolderMutation, ScanIngestDropFolderMutationVariables>;
export const EnqueueIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"EnqueueIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"EnqueueIngestAnalysisInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"enqueueIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<EnqueueIngestAnalysisMutation, EnqueueIngestAnalysisMutationVariables>;
export const RequeueIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RequeueIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"force"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Boolean"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"requeueIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"force"},"value":{"kind":"Variable","name":{"kind":"Name","value":"force"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<RequeueIngestAnalysisMutation, RequeueIngestAnalysisMutationVariables>;
export const PauseIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"PauseIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"pauseIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"jobId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<PauseIngestAnalysisMutation, PauseIngestAnalysisMutationVariables>;
export const ResumeIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ResumeIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"resumeIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"jobId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<ResumeIngestAnalysisMutation, ResumeIngestAnalysisMutationVariables>;
export const RetryIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RetryIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"retryIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"jobId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<RetryIngestAnalysisMutation, RetryIngestAnalysisMutationVariables>;
export const CancelIngestAnalysisDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"CancelIngestAnalysis"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"cancelIngestAnalysis"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"jobId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"jobId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<CancelIngestAnalysisMutation, CancelIngestAnalysisMutationVariables>;
export const DiscardIngestItemDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"DiscardIngestItem"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"reason"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"discardIngestItem"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"reason"},"value":{"kind":"Variable","name":{"kind":"Name","value":"reason"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<DiscardIngestItemMutation, DiscardIngestItemMutationVariables>;
export const ApplyIngestMetadataDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ApplyIngestMetadata"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ApplyIngestMetadataInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"applyIngestMetadata"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"dropItem"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<ApplyIngestMetadataMutation, ApplyIngestMetadataMutationVariables>;
export const BulkApplyIngestMetadataDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"BulkApplyIngestMetadata"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"BulkApplyIngestMetadataInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"bulkApplyIngestMetadata"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"applied"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"failures"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"message"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<BulkApplyIngestMetadataMutation, BulkApplyIngestMetadataMutationVariables>;
export const ApproveIngestItemDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ApproveIngestItem"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"strategy"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"MergeStrategy"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"approveIngestItem"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"strategy"},"value":{"kind":"Variable","name":{"kind":"Name","value":"strategy"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<ApproveIngestItemMutation, ApproveIngestItemMutationVariables>;
export const RejectIngestItemDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RejectIngestItem"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"reason"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"rejectIngestItem"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"reason"},"value":{"kind":"Variable","name":{"kind":"Name","value":"reason"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"IngestDropItemFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"IngestDropItemFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IngestDropItem"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"createdBy"}},{"kind":"Field","name":{"kind":"Name","value":"filename"}},{"kind":"Field","name":{"kind":"Name","value":"relativePath"}},{"kind":"Field","name":{"kind":"Name","value":"sizeBytes"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"mediaType"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"revision"}},{"kind":"Field","name":{"kind":"Name","value":"pendingFields"}},{"kind":"Field","name":{"kind":"Name","value":"error"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJob"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"jobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"priorityScore"}},{"kind":"Field","name":{"kind":"Name","value":"attempt"}},{"kind":"Field","name":{"kind":"Name","value":"queuedAt"}},{"kind":"Field","name":{"kind":"Name","value":"startedAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}},{"kind":"Field","name":{"kind":"Name","value":"qualityReport"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"algorithmVersion"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"generatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"checks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"weight"}},{"kind":"Field","name":{"kind":"Name","value":"normalizedScore"}},{"kind":"Field","name":{"kind":"Name","value":"contribution"}},{"kind":"Field","name":{"kind":"Name","value":"evidence"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadataCandidates"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"media"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<RejectIngestItemMutation, RejectIngestItemMutationVariables>;
export const SetIngestProviderSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetIngestProviderSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SetIngestProviderSettingsInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setIngestProviderSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"provider"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"version"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"capabilities"}},{"kind":"Field","name":{"kind":"Name","value":"supportedMediaTypes"}},{"kind":"Field","name":{"kind":"Name","value":"enabledByDefault"}},{"kind":"Field","name":{"kind":"Name","value":"requiresApiToken"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"optedIn"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"value"}}]}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<SetIngestProviderSettingsMutation, SetIngestProviderSettingsMutationVariables>;
export const VerifyIngestProviderDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"VerifyIngestProvider"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"settings"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"JSON"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"verifyIngestProvider"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"providerId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}}},{"kind":"Argument","name":{"kind":"Name","value":"settings"},"value":{"kind":"Variable","name":{"kind":"Name","value":"settings"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"responseStatus"}},{"kind":"Field","name":{"kind":"Name","value":"isValid"}},{"kind":"Field","name":{"kind":"Name","value":"error"}}]}}]}}]} as unknown as DocumentNode<VerifyIngestProviderMutation, VerifyIngestProviderMutationVariables>;
export const MetadataProviderConfigsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"MetadataProviderConfigs"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"metadataProviderConfigs"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"providerType"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<MetadataProviderConfigsQuery, MetadataProviderConfigsQueryVariables>;
export const CreateMetadataProviderDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"CreateMetadataProvider"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"CreateMetadataProviderConfigInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"createMetadataProvider"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"providerType"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}}]}}]}}]} as unknown as DocumentNode<CreateMetadataProviderMutation, CreateMetadataProviderMutationVariables>;
export const UpdateMetadataProviderDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"UpdateMetadataProvider"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"PatchMetadataProviderConfigInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"updateMetadataProvider"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"providerType"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}}]}}]}}]} as unknown as DocumentNode<UpdateMetadataProviderMutation, UpdateMetadataProviderMutationVariables>;
export const DeleteMetadataProviderDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"DeleteMetadataProvider"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deleteMetadataProvider"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}}]}}]}}]} as unknown as DocumentNode<DeleteMetadataProviderMutation, DeleteMetadataProviderMutationVariables>;
export const SetIngestQualityCheckSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetIngestQualityCheckSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SetIngestQualityCheckSettingsInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setIngestQualityCheckSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"checkId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"configured"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"value"}}]}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<SetIngestQualityCheckSettingsMutation, SetIngestQualityCheckSettingsMutationVariables>;
export const IngestProgressDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"subscription","name":{"kind":"Name","value":"IngestProgress"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"analysisJobId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"afterEventId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestProgress"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"analysisJobId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"analysisJobId"}}},{"kind":"Argument","name":{"kind":"Name","value":"afterEventId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"afterEventId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"eventId"}},{"kind":"Field","name":{"kind":"Name","value":"emittedAt"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"analysisJobId"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"phase"}},{"kind":"Field","name":{"kind":"Name","value":"completed"}},{"kind":"Field","name":{"kind":"Name","value":"total"}},{"kind":"Field","name":{"kind":"Name","value":"score"}},{"kind":"Field","name":{"kind":"Name","value":"message"}}]}}]}}]} as unknown as DocumentNode<IngestProgressSubscription, IngestProgressSubscriptionVariables>;
export const IngestProviderSearchDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"IngestProviderSearch"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"query"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaKind"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"IngestMediaKind"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"providers"}},"type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"limit"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"Int"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ingestProviderSearch"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"query"},"value":{"kind":"Variable","name":{"kind":"Name","value":"query"}}},{"kind":"Argument","name":{"kind":"Name","value":"mediaKind"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaKind"}}},{"kind":"Argument","name":{"kind":"Name","value":"providers"},"value":{"kind":"Variable","name":{"kind":"Name","value":"providers"}}},{"kind":"Argument","name":{"kind":"Name","value":"limit"},"value":{"kind":"Variable","name":{"kind":"Name","value":"limit"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"providerId"}},{"kind":"Field","name":{"kind":"Name","value":"externalId"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"year"}},{"kind":"Field","name":{"kind":"Name","value":"coverUrl"}},{"kind":"Field","name":{"kind":"Name","value":"summary"}},{"kind":"Field","name":{"kind":"Name","value":"score"}}]}}]}}]} as unknown as DocumentNode<IngestProviderSearchQuery, IngestProviderSearchQueryVariables>;
export const LookupIngestCandidateDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"LookupIngestCandidate"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"externalId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"lookupIngestCandidate"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"dropItemId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"dropItemId"}}},{"kind":"Argument","name":{"kind":"Name","value":"providerId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"providerId"}}},{"kind":"Argument","name":{"kind":"Name","value":"externalId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"externalId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"dropItemId"}},{"kind":"Field","name":{"kind":"Name","value":"provider"}},{"kind":"Field","name":{"kind":"Name","value":"providerVersion"}},{"kind":"Field","name":{"kind":"Name","value":"model"}},{"kind":"Field","name":{"kind":"Name","value":"confidence"}},{"kind":"Field","name":{"kind":"Name","value":"fields"}},{"kind":"Field","name":{"kind":"Name","value":"fieldConfidences"}},{"kind":"Field","name":{"kind":"Name","value":"sourceSha256"}},{"kind":"Field","name":{"kind":"Name","value":"provenance"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}}]}}]}}]} as unknown as DocumentNode<LookupIngestCandidateMutation, LookupIngestCandidateMutationVariables>;