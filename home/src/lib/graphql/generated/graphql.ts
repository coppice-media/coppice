/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };
import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';
/**
 * Narrows the cross-book annotation hub. Every field is a conjunction; an
 * empty list is the same as omitting it.
 */
export type AnnotationFilterInput = {
  kind?: Array<AnnotationKind> | null | undefined;
  /** Every book of one library */
  libraryId?: string | number | null | undefined;
  /** One book, by its Stump media id */
  mediaId?: string | number | null | undefined;
  /**
   * Case-insensitive substring match over the selected passage, the note,
   * and the book title
   */
  query?: string | null | undefined;
  /** Every book of one series */
  seriesId?: string | number | null | undefined;
  /** Only annotations created at or after this instant */
  since?: string | null | undefined;
  /**
   * Where the annotation came from, as reported by
   * [`AnnotationEntry::source`](crate::object::annotation::AnnotationEntry)
   */
  source?: Array<DeviceKind> | null | undefined;
};

/**
 * What an annotation is.
 *
 * A native `media_annotations` row always carries a Readium locator, so the
 * kind follows the locator's `text.highlight`: a row with that selected
 * passage is a `HIGHLIGHT`, a row carrying only the user's text is a `NOTE`.
 * Native `bookmarks` rows and liseur `bookmark` records are `BOOKMARK`.
 */
export type AnnotationKind =
  | 'BOOKMARK'
  | 'HIGHLIGHT'
  | 'NOTE';

/**
 * Represents a collected issue/series within a TPB or GN
 * See https://github.com/mylar3/mylar3/wiki/series.json-schema-%28version-1.0.1%29
 */
export type CollectedItemInput = {
  /** CV ComicID of series */
  comicid?: string | null | undefined;
  /** CV IssueID of single issue (not valid if multiple issues) */
  issueid?: string | null | undefined;
  /** Listing of issue numbers present pertaining to related comicid in collection */
  issues?: string | null | undefined;
  /** The title of the series */
  series?: string | null | undefined;
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

export type CreateOrUpdateLibraryInput = {
  config?: LibraryConfigInput | null | undefined;
  description?: string | null | undefined;
  emoji?: string | null | undefined;
  name: string;
  path: string;
  scanAfterPersist?: boolean;
  tags?: Array<string> | null | undefined;
};

/** A simple cursor-based pagination input object */
export type CursorPagination = {
  after?: string | null | undefined;
  limit?: number;
};

/** The storage a device credential references */
export type DeviceCredentialKind =
  /** `credential_ref` is an `api_keys.short_token` */
  | 'API_KEY'
  /** `credential_ref` is a `liseur_sync_tokens.id` */
  | 'LISEUR_TOKEN'
  /** `credential_ref` is a `sessions.session_id` */
  | 'SESSION';

/**
 * The client family a registered device belongs to. The kind decides which
 * credential is minted for the device and which endpoints it is handed.
 */
export type DeviceKind =
  /** A script or integration using the native API */
  | 'API'
  /** A Kobo eReader using the native Kobo sync protocol */
  | 'KOBO'
  /** Komelia using the Komga-compatible profile */
  | 'KOMELIA'
  /** A KOReader install using the KOReader progress sync protocol */
  | 'KOREADER'
  /** Liseur using the native liseur-sync protocol */
  | 'LISEUR'
  /** Mihon (Tachiyomi) using the Komga-compatible profile */
  | 'MIHON'
  /** A generic OPDS reader */
  | 'OPDS'
  /** A browser session */
  | 'WEB';

/**
 * The lifecycle state of a device-pairing request. `Expired` is derived from
 * `expires_at` for pending rows and persisted lazily once observed, so a row's
 * stored status may still read `Pending` after the deadline; always go through
 * [`Model::effective_status`].
 */
export type DevicePairingStatus =
  | 'APPROVED'
  | 'DENIED'
  | 'EXPIRED'
  | 'PENDING';

/** The wire protocol through which a device credential was exercised */
export type DeviceProtocol =
  | 'API'
  | 'KOBO'
  | 'KOMGA'
  | 'KOREADER'
  | 'LISEUR'
  | 'OPDS';

export type Dimension =
  | 'HEIGHT'
  | 'WIDTH';

export type EpubProgressInput = {
  deviceId?: string | null | undefined;
  elapsedSecondsDelta?: number | null | undefined;
  isComplete?: boolean | null | undefined;
  locator: ReadiumLocatorInput;
  percentage?: unknown;
  resetElapsedSeconds?: boolean | null | undefined;
};

/**
 * A resize option which will resize the image to the given dimensions, without
 * maintaining the aspect ratio.
 */
export type ExactDimensionResizeInput = {
  /** The height (in pixels) the resulting image should be resized to */
  height: number;
  /** The width (in pixels) the resulting image should be resized to */
  width: number;
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

/**
 * A resize option which will resize the image to fit within the given dimensions,
 * maintaining the aspect ratio.
 *
 * If the image already fits within the dimensions, it will not be scaled up.
 */
export type FitWithinResizeInput = {
  /** The maximum height (in pixels) of the resulting image */
  height: number;
  /** The maximum width (in pixels) of the resulting image */
  width: number;
};

/** Options for processing images throughout Stump. */
export type ImageProcessorOptionsInput = {
  /** The format to use when generating an image. See [`SupportedImageFormat`] */
  format: SupportedImageFormat;
  /** The page to use when generating an image. This is not applicable to all media formats. */
  page?: number | null | undefined;
  /**
   * The quality to use when generating an image. This is a number between 1 and 100,
   * where 100 is the highest quality. Omitting this value will use the default quality
   * of 100.
   */
  quality?: number | null | undefined;
  /** The size factor to use when generating an image. See [`ImageResizeOptions`] */
  resizeMethod?: ImageResizeMethodInput | null | undefined;
};

/** The resize options to use when generating an image */
export type ImageResizeMethodInput =
  {   exact: ExactDimensionResizeInput; fitWithin?: never; scaleDimension?: never; scaleEvenlyByFactor?: never; }
  |  { exact?: never;   fitWithin: FitWithinResizeInput; scaleDimension?: never; scaleEvenlyByFactor?: never; }
  |  { exact?: never; fitWithin?: never;   scaleDimension: ScaledDimensionResizeInput; scaleEvenlyByFactor?: never; }
  |  { exact?: never; fitWithin?: never; scaleDimension?: never;   scaleEvenlyByFactor: ScaleEvenlyByFactorInput; };

export type IngestSettingValueType =
  | 'BOOLEAN'
  | 'INTEGER'
  | 'JSON'
  | 'NUMBER'
  | 'STRING';

export type JobStatus =
  | 'CANCELLED'
  | 'COMPLETED'
  | 'FAILED'
  | 'PAUSED'
  | 'QUEUED'
  | 'RUNNING';

export type LibraryConfigInput = {
  convertRarToZip: boolean;
  defaultLibraryViewMode: LibraryViewMode;
  defaultReadingDir: ReadingDirection;
  defaultReadingImageScaleFit: ReadingImageScaleFit;
  defaultReadingMode: ReadingMode;
  generateFileHashes: boolean;
  generateKoreaderHashes: boolean;
  hardDeleteConversions: boolean;
  hideSeriesView: boolean;
  ignoreRules?: Array<string> | null | undefined;
  libraryPattern: LibraryPattern;
  libraryType: LibraryType;
  processMetadata: boolean;
  processThumbnailColorsEvenWithoutConfig: boolean;
  skipBookOverview: boolean;
  thumbnailConfig?: ImageProcessorOptionsInput | null | undefined;
  watch: boolean;
};

export type LibraryFilterInput = {
  _and?: Array<LibraryFilterInput> | null | undefined;
  _not?: Array<LibraryFilterInput> | null | undefined;
  _or?: Array<LibraryFilterInput> | null | undefined;
  id?: FieldFilterString | null | undefined;
  name?: FieldFilterString | null | undefined;
  path?: FieldFilterString | null | undefined;
};

/** The different patterns a library may be organized by */
export type LibraryPattern =
  | 'COLLECTION_BASED'
  | 'SERIES_BASED';

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

export type LibraryViewMode =
  | 'BOOKS'
  | 'SERIES';

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

export type MediaMetadataModelOrdering =
  | 'AGE_RATING'
  | 'CHARACTERS'
  | 'COLORISTS'
  | 'COVER_ARTISTS'
  | 'DAY'
  | 'EDITORS'
  | 'FORMAT'
  | 'GENRES'
  | 'ID'
  | 'IDENTIFIER_AMAZON'
  | 'IDENTIFIER_CALIBRE'
  | 'IDENTIFIER_GOOGLE'
  | 'IDENTIFIER_ISBN'
  | 'IDENTIFIER_MOBI_ASIN'
  | 'IDENTIFIER_UUID'
  | 'INKERS'
  | 'LANGUAGE'
  | 'LETTERERS'
  | 'LINKS'
  | 'LOCKED_FIELDS'
  | 'MEDIA_ID'
  | 'METADATA_EXTERNAL_ID'
  | 'METADATA_SOURCE'
  | 'MONTH'
  | 'NOTES'
  | 'NUMBER'
  | 'PAGE_COUNT'
  | 'PENCILLERS'
  | 'PUBLISHER'
  | 'SERIES'
  | 'SERIES_GROUP'
  | 'STORY_ARC'
  | 'STORY_ARC_NUMBER'
  | 'SUMMARY'
  | 'TEAMS'
  | 'TITLE'
  | 'TITLE_SORT'
  | 'VOLUME'
  | 'WRITERS'
  | 'YEAR';

export type MediaMetadataOrderByField = {
  direction: OrderDirection;
  field: MediaMetadataModelOrdering;
};

export type MediaModelOrdering =
  | 'CREATED_AT'
  | 'DELETED_AT'
  | 'EXTENSION'
  | 'HASH'
  | 'ID'
  | 'KOREADER_HASH'
  | 'MODIFIED_AT'
  | 'NAME'
  | 'PAGES'
  | 'PATH'
  | 'REMOTE_CHAPTER_ID'
  | 'REMOTE_ID'
  | 'SERIES_ID'
  | 'SIZE'
  | 'SOURCE_PROVIDER'
  | 'STATUS'
  | 'THUMBNAIL_META'
  | 'THUMBNAIL_PATH'
  | 'UPDATED_AT';

export type MediaOrderBy =
  {   media: MediaOrderByField; metadata?: never; }
  |  { media?: never;   metadata: MediaMetadataOrderByField; };

export type MediaOrderByField = {
  direction: OrderDirection;
  field: MediaModelOrdering;
};

export type MediaProgressInput =
  {   epub: EpubProgressInput; paged?: never; }
  |  { epub?: never;   paged: PagedProgressInput; };

/**
 * The events a user can route to a channel. Persisted by name in
 * `notification_rules.event_kind` and in queued dispatch jobs, so variants are
 * append-only.
 */
export type NotificationKind =
  /** A staged analysis job failed */
  | 'ANALYSIS_JOB_FAILED'
  /** A registered device authenticated for the first time */
  | 'DEVICE_FIRST_SEEN'
  /** A pairing was approved and the device received its credential */
  | 'DEVICE_PAIRED'
  /** A staged ingest item finished analysis and needs a human decision */
  | 'INGEST_AWAITING_REVIEW'
  /** Provider lookup finished for a staged item */
  | 'PROVIDER_MATCH_DONE'
  /** A quality report contains at least one failed check */
  | 'QUALITY_FAILED'
  /** A library scan completed */
  | 'SCAN_FINISHED'
  /** Sent by `testNotificationChannel`; never routed by rules */
  | 'TEST';

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

export type OrderDirection =
  | 'ASC'
  | 'DESC';

export type PagedProgressInput = {
  deviceId?: string | null | undefined;
  elapsedSecondsDelta?: number | null | undefined;
  page: number;
  resetElapsedSeconds?: boolean | null | undefined;
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

/** The different reading directions supported by any Stump reader */
export type ReadingDirection =
  | 'LTR'
  | 'RTL';

/** The different ways an image may be scaled to fit a reader's viewport */
export type ReadingImageScaleFit =
  | 'AUTO'
  | 'HEIGHT'
  | 'NONE'
  | 'WIDTH';

/** The different reading modes supported by any Stump reader */
export type ReadingMode =
  | 'CONTINUOUS_HORIZONTAL'
  | 'CONTINUOUS_VERTICAL'
  | 'PAGED';

/**
 * How far back reading statistics reach, counted in logical reading days
 * ending today.
 */
export type ReadingStatsSpan =
  | 'ALL_TIME'
  /** Today only */
  | 'DAY'
  /** The last 30 days */
  | 'MONTH'
  /** The last 90 days */
  | 'QUARTER'
  /** The last 7 days */
  | 'WEEK'
  /** The last 365 days */
  | 'YEAR';

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

export type ReadiumLocationInput = {
  cssSelector?: string | null | undefined;
  fragments?: Array<string> | null | undefined;
  partialCfi?: string | null | undefined;
  position?: number | null | undefined;
  progression?: unknown;
  totalProgression?: unknown;
};

export type ReadiumLocatorInput = {
  chapterTitle?: string;
  href: string;
  koboSpan?: string | null | undefined;
  locations?: ReadiumLocationInput | null | undefined;
  text?: ReadiumTextInput | null | undefined;
  title?: string | null | undefined;
  type?: string;
};

export type ReadiumTextInput = {
  after?: string | null | undefined;
  before?: string | null | undefined;
  highlight?: string | null | undefined;
};

export type ScaleEvenlyByFactorInput = {
  /**
   * The factor to scale the image by. Note that this was made a [Decimal]
   * to correct precision issues
   */
  factor: unknown;
};

/**
 * A resize option which will resize the image while maintaining the aspect ratio.
 * The dimension *not* specified will be calculated based on the aspect ratio.
 */
export type ScaledDimensionResizeInput = {
  /** The dimension to set with the given size, e.g. `Height` or `Width`. */
  dimension: Dimension;
  /** The size (in pixels) to set the specified dimension to. */
  size: number;
};

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

export type SeriesMetadataInput = {
  ageRating?: number | null | undefined;
  booktype?: string | null | undefined;
  characters?: Array<string> | null | undefined;
  collects?: Array<CollectedItemInput> | null | undefined;
  comicImage?: string | null | undefined;
  comicid?: number | null | undefined;
  descriptionFormatted?: string | null | undefined;
  genres?: Array<string> | null | undefined;
  imprint?: string | null | undefined;
  links?: Array<string> | null | undefined;
  metaType?: string | null | undefined;
  publicationRun?: string | null | undefined;
  publisher?: string | null | undefined;
  status?: string | null | undefined;
  summary?: string | null | undefined;
  title?: string | null | undefined;
  totalIssues?: number | null | undefined;
  volume?: number | null | undefined;
  writers?: Array<string> | null | undefined;
  year?: number | null | undefined;
};

export type SeriesMetadataModelOrdering =
  | 'AGE_RATING'
  | 'ALTERNATE_TITLES'
  | 'ALTERNATE_TITLES_LOCK'
  | 'BOOKTYPE'
  | 'CHARACTERS'
  | 'COLLECTS'
  | 'COMICID'
  | 'COMIC_IMAGE'
  | 'DESCRIPTION_FORMATTED'
  | 'GENRES'
  | 'IMPRINT'
  | 'LANGUAGE'
  | 'LANGUAGE_LOCK'
  | 'LINKS'
  | 'LOCKED_FIELDS'
  | 'METADATA_EXTERNAL_ID'
  | 'METADATA_SOURCE'
  | 'META_TYPE'
  | 'PUBLICATION_RUN'
  | 'PUBLISHER'
  | 'READING_DIRECTION'
  | 'READING_DIRECTION_LOCK'
  | 'SERIES_ID'
  | 'STATUS'
  | 'SUMMARY'
  | 'TITLE'
  | 'TITLE_SORT'
  | 'TITLE_SORT_LOCK'
  | 'TOTAL_ISSUES'
  | 'VOLUME'
  | 'WRITERS'
  | 'YEAR';

export type SeriesMetadataOrderByField = {
  direction: OrderDirection;
  field: SeriesMetadataModelOrdering;
};

export type SeriesModelOrdering =
  | 'CREATED_AT'
  | 'DELETED_AT'
  | 'DESCRIPTION'
  | 'ID'
  | 'LIBRARY_ID'
  | 'NAME'
  | 'PATH'
  | 'REMOTE_ID'
  | 'SOURCE_PROVIDER'
  | 'STATUS'
  | 'THUMBNAIL_META'
  | 'THUMBNAIL_PATH'
  | 'UPDATED_AT';

export type SeriesOrderBy =
  {   metadata: SeriesMetadataOrderByField; series?: never; }
  |  { metadata?: never;   series: SeriesOrderByField; };

export type SeriesOrderByField = {
  direction: OrderDirection;
  field: SeriesModelOrdering;
};

export type SetNotificationChannelSettingsInput = {
  /** The channel whose per-user settings are written. */
  channelId: string;
  /**
   * Values keyed by the channel's settings definition keys. Stored values
   * are merged over the channel defaults; omitted secret keys keep their
   * previously stored (encrypted) value.
   */
  settings: unknown;
};

/** Supported image formats for processing images throughout Stump */
export type SupportedImageFormat =
  | 'JPEG'
  | 'PNG'
  | 'WEBP';

/**
 * A simple pagination input object which does not paginate. An explicit struct is
 * required as a limitation of async_graphql's [OneofObject], which doesn't allow
 * for empty variants.
 */
export type Unpaginated = {
  unpaginated: boolean;
};

export type UpdateAnnotationInput = {
  annotationText?: string | null | undefined;
  id: string;
};

/** The permissions a user may be granted */
export type UserPermission =
  /** Grant access to read/create their own API keys */
  | 'ACCESS_API_KEYS'
  /**
   * TODO: Expand permissions for bookclub + smartlist
   * Grant access to the book club feature
   */
  | 'ACCESS_BOOK_CLUB'
  /** Grant access to the kobo sync feature */
  | 'ACCESS_KOBO_SYNC'
  /** Grant access to the koreader sync feature */
  | 'ACCESS_KOREADER_SYNC'
  /** Grant access to access the smart list feature. This includes the ability to create and edit smart lists */
  | 'ACCESS_SMART_LIST'
  /** Grant user access to change **their own** avatar */
  | 'CHANGE_AVATAR'
  /** Grant user access to change **their own** password */
  | 'CHANGE_PASSWORD'
  /** Grant user access to change **their own** username */
  | 'CHANGE_USERNAME'
  /** Grant access to create a book club (access book club) */
  | 'CREATE_BOOK_CLUB'
  /** Grant access to create a library */
  | 'CREATE_LIBRARY'
  /** Grant access to create a notifier */
  | 'CREATE_NOTIFIER'
  /** Grant access to delete the library (manage library) */
  | 'DELETE_LIBRARY'
  /** Grant access to delete a notifier */
  | 'DELETE_NOTIFIER'
  /** Grant access to download files from a library */
  | 'DOWNLOAD_FILE'
  /** Grant access to edit basic details about the library */
  | 'EDIT_LIBRARY'
  /**
   * Grants access to edit any existing metadata for media/series. This will only
   * be applied to the database-level metadata.
   */
  | 'EDIT_METADATA'
  /** Grant access to edit thumbnails for media/series */
  | 'EDIT_THUMBNAILS'
  /** Grant access to create an emailer */
  | 'EMAILER_CREATE'
  /** Grant access to manage an emailer */
  | 'EMAILER_MANAGE'
  /** Grant access to read any emailers in the system */
  | 'EMAILER_READ'
  /** Grant access to send an arbitrary email, bypassing any registered device requirements */
  | 'EMAIL_ARBITRARY_SEND'
  /** Grant access to send an email */
  | 'EMAIL_SEND'
  /** Grant access to access the file explorer */
  | 'FILE_EXPLORER'
  /** Grant access to manage jobs, like pausing, resuming, deleting, or cancelling them */
  | 'MANAGE_JOBS'
  /** Grant access to manage the library (scan,edit,manage relations) */
  | 'MANAGE_LIBRARY'
  /** Grant access to manage a notifier */
  | 'MANAGE_NOTIFIER'
  /** Grant access to manage the server. This is effectively a step below server owner */
  | 'MANAGE_SERVER'
  /** Grant access to manage users (create,edit,delete) */
  | 'MANAGE_USERS'
  /** Grant access to manage metadata fetch statuses (accept matches, etc) */
  | 'METADATA_FETCH_RECORD_MANAGE'
  /** Grant access to read metadata fetch statuses */
  | 'METADATA_FETCH_RECORD_READ'
  /** Grant access to manage metadata provider configurations (create, update, delete) */
  | 'METADATA_PROVIDER_MANAGE'
  /** Grant access to read metadata provider configurations */
  | 'METADATA_PROVIDER_READ'
  /** Grant access to read jobs */
  | 'READ_JOBS'
  /** Grant access to read notifiers */
  | 'READ_NOTIFIER'
  /** Grant access to read application-level logs, e.g. job logs */
  | 'READ_PERSISTED_LOGS'
  /** Grant access to read system logs */
  | 'READ_SYSTEM_LOGS'
  /**
   * Grant access to read users.
   *
   * Note that this is explicitly for querying users via user-specific endpoints.
   * This would not affect relational queries, such as members in a common book club.
   */
  | 'READ_USERS'
  /** Grant access to scan the library for new files */
  | 'SCAN_LIBRARY'
  /** Grant access to upload files to a library */
  | 'UPLOAD_FILE'
  /**
   * Grants access to write back the database-level metadata for media/series.
   * This should be treated with caution, as technically it would allow for
   * overwriting existing metadata at the file-level
   */
  | 'WRITE_BACK_METADATA';

export type ConsoleAnnotationFieldsFragment = { id: string, kind: AnnotationKind, source: DeviceKind, sourceDeviceId: string | null, sourceDeviceName: string | null, editable: boolean, chapterTitle: string | null, href: string | null, fragment: string | null, page: number | null, progression: number | null, excerpt: string | null, note: string | null, color: string | null, createdAt: string | null, updatedAt: string | null, book: { key: string, mediaId: string | null, title: string, authors: Array<string>, seriesId: string | null, seriesName: string | null, libraryId: string | null, extension: string | null } };

export type ConsoleAnnotationsQueryVariables = Exact<{
  filter?: AnnotationFilterInput | null | undefined;
  pagination?: OffsetPagination | null | undefined;
}>;


export type ConsoleAnnotationsQuery = { annotations: { total: number, bookCount: number, hasNext: boolean, items: Array<{ id: string, kind: AnnotationKind, source: DeviceKind, sourceDeviceId: string | null, sourceDeviceName: string | null, editable: boolean, chapterTitle: string | null, href: string | null, fragment: string | null, page: number | null, progression: number | null, excerpt: string | null, note: string | null, color: string | null, createdAt: string | null, updatedAt: string | null, book: { key: string, mediaId: string | null, title: string, authors: Array<string>, seriesId: string | null, seriesName: string | null, libraryId: string | null, extension: string | null } }> } };

export type ConsoleAnnotationBooksQueryVariables = Exact<{ [key: string]: never; }>;


export type ConsoleAnnotationBooksQuery = { annotations: { items: Array<{ book: { key: string, mediaId: string | null, title: string } }> } };

export type ConsoleUpdateAnnotationMutationVariables = Exact<{
  input: UpdateAnnotationInput;
}>;


export type ConsoleUpdateAnnotationMutation = { updateAnnotation: { id: string, annotationText: string | null, updatedAt: string } };

export type ConsoleDeleteAnnotationMutationVariables = Exact<{
  id: string;
}>;


export type ConsoleDeleteAnnotationMutation = { deleteAnnotation: { id: string } };

export type ConsoleAnnotationSinksQueryVariables = Exact<{ [key: string]: never; }>;


export type ConsoleAnnotationSinksQuery = { annotationSinks: Array<{ id: string, name: string, description: string, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null, helpUrl: string | null }> }>, annotationSyncStatus: { userId: string, pending: boolean, sinks: Array<{ sinkId: string, enabled: boolean, lastRunAt: string | null, lastError: string | null }> } };

export type ConsoleSetAnnotationSinkSettingsMutationVariables = Exact<{
  sinkId: string;
  settings?: unknown;
  enabled?: boolean | null | undefined;
}>;


export type ConsoleSetAnnotationSinkSettingsMutation = { setAnnotationSinkSettings: { userId: string, pending: boolean, sinks: Array<{ sinkId: string, enabled: boolean, lastRunAt: string | null, lastError: string | null }> } };

export type ConsoleRunAnnotationSyncMutationVariables = Exact<{ [key: string]: never; }>;


export type ConsoleRunAnnotationSyncMutation = { runAnnotationSync: { userId: string, pending: boolean, sinks: Array<{ sinkId: string, enabled: boolean, lastRunAt: string | null, lastError: string | null }> } };

export type DashboardViewerQueryVariables = Exact<{ [key: string]: never; }>;


export type DashboardViewerQuery = { me: { id: string, username: string, isServerOwner: boolean, permissions: Array<UserPermission> } };

export type DashboardBookCardFragment = { id: string, resolvedName: string, extension: string, pages: number, createdAt: string, seriesId: string | null, thumbnail: { url: string }, series: { id: string, resolvedName: string }, readProgress: { page: number | null, percentageCompleted: unknown, elapsedSeconds: number, updatedAt: string | null } | null };

export type DashboardKeepReadingQueryVariables = Exact<{
  pagination: Pagination;
}>;


export type DashboardKeepReadingQuery = { keepReading: { nodes: Array<{ id: string, resolvedName: string, extension: string, pages: number, createdAt: string, seriesId: string | null, thumbnail: { url: string }, series: { id: string, resolvedName: string }, readProgress: { page: number | null, percentageCompleted: unknown, elapsedSeconds: number, updatedAt: string | null } | null }> } };

export type DashboardRecentlyAddedQueryVariables = Exact<{
  pagination: Pagination;
}>;


export type DashboardRecentlyAddedQuery = { recentlyAddedMedia: { nodes: Array<{ id: string, resolvedName: string, extension: string, pages: number, createdAt: string, seriesId: string | null, thumbnail: { url: string }, series: { id: string, resolvedName: string }, readProgress: { page: number | null, percentageCompleted: unknown, elapsedSeconds: number, updatedAt: string | null } | null }> } };

export type DashboardJobsQueryVariables = Exact<{
  pagination: Pagination;
}>;


export type DashboardJobsQuery = { jobs: { nodes: Array<{ id: string, name: string, description: string | null, status: JobStatus, msElapsed: number, createdAt: string, completedAt: string | null }> } };

export type DashboardLiveEventsSubscriptionVariables = Exact<{ [key: string]: never; }>;


export type DashboardLiveEventsSubscription = { readEvents:
    | { __typename: 'AnalysisJobFailed' }
    | { __typename: 'CollectionAdded' }
    | { __typename: 'CollectionChanged' }
    | { __typename: 'CollectionDeleted' }
    | { __typename: 'CreatedManySeries' }
    | { __typename: 'CreatedMedia' }
    | { __typename: 'CreatedOrUpdatedManyMedia' }
    | { __typename: 'DevicePaired' }
    | { __typename: 'DevicePairingRequested' }
    | { __typename: 'DeviceSeen', deviceId: string, protocol: DeviceProtocol }
    | { __typename: 'DiscoveredMissingLibrary' }
    | { __typename: 'IngestAwaitingReview' }
    | { __typename: 'JobOutput', id: string }
    | { __typename: 'JobQueueStatus', count: number, countByType: unknown }
    | { __typename: 'JobStarted', id: string }
    | { __typename: 'JobUpdate', id: string, status: JobStatus | null, message: string | null, subtitle: string | null, completedTasks: number | null, remainingTasks: number | null }
    | { __typename: 'LibraryCreated' }
    | { __typename: 'LibraryDeleted' }
    | { __typename: 'LibraryUpdated' }
    | { __typename: 'MediaDeleted' }
    | { __typename: 'ProviderMatchDone' }
    | { __typename: 'QualityFailed' }
    | { __typename: 'ReadListAdded' }
    | { __typename: 'ReadListChanged' }
    | { __typename: 'ReadListDeleted' }
    | { __typename: 'SeriesDeleted' }
   };

export type NotificationSettingsQueryVariables = Exact<{ [key: string]: never; }>;


export type NotificationSettingsQuery = { notificationChannels: Array<{ id: string, label: string, settings: Array<{ key: string, label: string, valueType: IngestSettingValueType, required: boolean, secret: boolean, defaultValue: unknown, description: string | null, helpUrl: string | null }> }>, notificationRules: Array<{ id: number, eventKind: string, channelId: string, enabled: boolean }>, notificationChannelSettings: Array<{ channelId: string, values: unknown }> };

export type SetNotificationRuleMutationVariables = Exact<{
  eventKind: NotificationKind;
  channelId: string;
  enabled: boolean;
}>;


export type SetNotificationRuleMutation = { setNotificationRule: { id: number, eventKind: string, channelId: string, enabled: boolean } };

export type SetNotificationChannelSettingsMutationVariables = Exact<{
  input: SetNotificationChannelSettingsInput;
}>;


export type SetNotificationChannelSettingsMutation = { setNotificationChannelSettings: { channelId: string, values: unknown } };

export type TestNotificationChannelMutationVariables = Exact<{
  channelId: string;
}>;


export type TestNotificationChannelMutation = { testNotificationChannel: boolean };

export type ConsolePageInfoFragment = { totalItems: number, totalPages: number, currentPage: number, pageSize: number };

export type ConsoleLibraryCardFragment = { id: string, name: string, description: string | null, path: string, emoji: string | null, status: FileStatus, lastScannedAt: string | null, sourceProvider: string | null, config: { libraryType: LibraryType, libraryPattern: LibraryPattern, watch: boolean }, stats: { seriesCount: number, bookCount: number, completedBooks: number, inProgressBooks: number, totalBytes: number } };

export type ConsoleSeriesCardFragment = { id: string, name: string, resolvedName: string, path: string, status: FileStatus, mediaCount: number, readCount: number, unreadCount: number, percentageCompleted: number, isComplete: boolean, sourceProvider: string | null, thumbnail: { url: string }, metadata: { title: string | null, publisher: string | null } | null, tags: Array<{ id: number, name: string }> };

export type ConsoleBookRowFragment = { id: string, name: string, resolvedName: string, extension: string, pages: number, size: number, status: FileStatus, seriesId: string | null, path: string, series: { id: string, resolvedName: string }, thumbnail: { url: string }, metadata: { title: string | null, publisher: string | null, writers: Array<string> } | null, tags: Array<{ id: number, name: string }>, readProgress: { page: number | null, percentageCompleted: unknown, updatedAt: string | null } | null, readHistory: Array<{ readthroughNumber: number, completedAt: string, dnf: boolean }> };

export type ConsoleLibrariesQueryVariables = Exact<{
  pagination: Pagination;
}>;


export type ConsoleLibrariesQuery = { libraries: { nodes: Array<{ id: string, name: string, description: string | null, path: string, emoji: string | null, status: FileStatus, lastScannedAt: string | null, sourceProvider: string | null, config: { libraryType: LibraryType, libraryPattern: LibraryPattern, watch: boolean }, stats: { seriesCount: number, bookCount: number, completedBooks: number, inProgressBooks: number, totalBytes: number } }>, pageInfo:
      | { totalItems: number, totalPages: number, currentPage: number, pageSize: number }
      | Record<PropertyKey, never>
     } };

export type ConsoleLibraryOptionsQueryVariables = Exact<{ [key: string]: never; }>;


export type ConsoleLibraryOptionsQuery = { libraries: { nodes: Array<{ id: string, name: string, emoji: string | null }> } };

export type ConsoleLibraryDetailQueryVariables = Exact<{
  id: string | number;
}>;


export type ConsoleLibraryDetailQuery = { libraryById: { id: string, name: string, description: string | null, path: string, emoji: string | null, status: FileStatus, lastScannedAt: string | null, sourceProvider: string | null, tags: Array<{ id: number, name: string }>, config: { libraryType: LibraryType, libraryPattern: LibraryPattern, watch: boolean }, stats: { seriesCount: number, bookCount: number, completedBooks: number, inProgressBooks: number, totalBytes: number } } | null };

export type ConsoleLibrarySeriesQueryVariables = Exact<{
  filter: SeriesFilterInput;
  orderBy: Array<SeriesOrderBy> | SeriesOrderBy;
  pagination: Pagination;
}>;


export type ConsoleLibrarySeriesQuery = { series: { nodes: Array<{ id: string, name: string, resolvedName: string, path: string, status: FileStatus, mediaCount: number, readCount: number, unreadCount: number, percentageCompleted: number, isComplete: boolean, sourceProvider: string | null, thumbnail: { url: string }, metadata: { title: string | null, publisher: string | null } | null, tags: Array<{ id: number, name: string }> }>, pageInfo:
      | { totalItems: number, totalPages: number, currentPage: number, pageSize: number }
      | Record<PropertyKey, never>
     } };

export type ConsoleBooksQueryVariables = Exact<{
  filter: MediaFilterInput;
  orderBy: Array<MediaOrderBy> | MediaOrderBy;
  pagination: Pagination;
}>;


export type ConsoleBooksQuery = { media: { nodes: Array<{ id: string, name: string, resolvedName: string, extension: string, pages: number, size: number, status: FileStatus, seriesId: string | null, path: string, series: { id: string, resolvedName: string }, thumbnail: { url: string }, metadata: { title: string | null, publisher: string | null, writers: Array<string> } | null, tags: Array<{ id: number, name: string }>, readProgress: { page: number | null, percentageCompleted: unknown, updatedAt: string | null } | null, readHistory: Array<{ readthroughNumber: number, completedAt: string, dnf: boolean }> }>, pageInfo:
      | { totalItems: number, totalPages: number, currentPage: number, pageSize: number }
      | Record<PropertyKey, never>
     } };

export type ConsoleSeriesDetailQueryVariables = Exact<{
  id: string | number;
}>;


export type ConsoleSeriesDetailQuery = { seriesById: { description: string | null, resolvedDescription: string | null, libraryId: string | null, id: string, name: string, resolvedName: string, path: string, status: FileStatus, mediaCount: number, readCount: number, unreadCount: number, percentageCompleted: number, isComplete: boolean, sourceProvider: string | null, library: { id: string, name: string, config: { libraryType: LibraryType } }, stats: { bookCount: number, completedBooks: number, inProgressBooks: number, totalReadingTimeSeconds: number }, metadata: { ageRating: number | null, booktype: string | null, characters: Array<string>, comicImage: string | null, comicid: number | null, descriptionFormatted: string | null, genres: Array<string>, imprint: string | null, links: Array<string>, metaType: string | null, publicationRun: string | null, publisher: string | null, status: string | null, summary: string | null, title: string | null, totalIssues: number | null, volume: number | null, writers: Array<string>, year: number | null, collects: Array<{ series: string | null, comicid: string | null, issueid: string | null, issues: string | null }> } | null, thumbnail: { url: string }, tags: Array<{ id: number, name: string }> } | null };

export type ConsoleSeriesPickerQueryVariables = Exact<{
  filter: SeriesFilterInput;
}>;


export type ConsoleSeriesPickerQuery = { series: { nodes: Array<{ id: string, name: string, resolvedName: string, mediaCount: number, sourceProvider: string | null }> } };

export type ConsoleAuthorsQueryVariables = Exact<{
  search?: string | null | undefined;
  libraryId?: string | null | undefined;
  pagination: Pagination;
}>;


export type ConsoleAuthorsQuery = { authors: { nodes: Array<{ name: string, books: Array<{ id: string }>, series: Array<{ title: string }> }>, pageInfo:
      | { totalItems: number, totalPages: number, currentPage: number, pageSize: number }
      | Record<PropertyKey, never>
     } };

export type ConsoleLibraryPublishersQueryVariables = Exact<{
  id: string | number;
}>;


export type ConsoleLibraryPublishersQuery = { libraryById: { id: string, publishers: Array<string> } | null };

export type ConsoleTagsQueryVariables = Exact<{ [key: string]: never; }>;


export type ConsoleTagsQuery = { tags: Array<{ id: number, name: string, kind: string }> };

export type ConsoleEntityBookCountQueryVariables = Exact<{
  filter: MediaFilterInput;
}>;


export type ConsoleEntityBookCountQuery = { media: { pageInfo:
      | { totalItems: number, totalPages: number, currentPage: number, pageSize: number }
      | Record<PropertyKey, never>
     } };

export type ConsoleCreateLibraryMutationVariables = Exact<{
  input: CreateOrUpdateLibraryInput;
}>;


export type ConsoleCreateLibraryMutation = { createLibrary: { id: string, name: string, description: string | null, path: string, emoji: string | null, status: FileStatus, lastScannedAt: string | null, sourceProvider: string | null, config: { libraryType: LibraryType, libraryPattern: LibraryPattern, watch: boolean }, stats: { seriesCount: number, bookCount: number, completedBooks: number, inProgressBooks: number, totalBytes: number } } };

export type ConsoleScanLibraryMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleScanLibraryMutation = { scanLibrary: boolean };

export type ConsoleAnalyzeLibraryMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleAnalyzeLibraryMutation = { analyzeLibrary: boolean };

export type ConsoleFinishMediaMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleFinishMediaMutation = { finishMediaProgress: boolean };

export type ConsoleResetMediaProgressMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleResetMediaProgressMutation = { clearMediaProgress: boolean, deleteMediaReadingHistory: number };

export type ConsoleFinishSeriesMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleFinishSeriesMutation = { finishSeriesProgress: number };

export type ConsoleClearSeriesHistoryMutationVariables = Exact<{
  id: string | number;
}>;


export type ConsoleClearSeriesHistoryMutation = { clearSeriesReadingHistory: number };

export type ConsoleRenameSeriesMutationVariables = Exact<{
  id: string | number;
  input: SeriesMetadataInput;
}>;


export type ConsoleRenameSeriesMutation = { updateSeriesMetadata: { id: string, name: string, resolvedName: string, metadata: { title: string | null } | null } };

export type ConsoleMoveMediaToSeriesMutationVariables = Exact<{
  mediaIds: Array<string | number> | string | number;
  seriesId: string | number;
}>;


export type ConsoleMoveMediaToSeriesMutation = { moveMediaToSeries: { id: string, name: string, resolvedName: string, mediaCount: number } };

export type ConsoleMergeSeriesMutationVariables = Exact<{
  keep: string | number;
  drop: string | number;
}>;


export type ConsoleMergeSeriesMutation = { mergeSeries: { droppedSeriesId: string, moved: number, missingFiles: number, droppedDirectory: boolean, kept: { id: string, name: string, resolvedName: string, mediaCount: number } } };

export type ConsoleSplitSeriesMutationVariables = Exact<{
  mediaIds: Array<string | number> | string | number;
  name: string;
}>;


export type ConsoleSplitSeriesMutation = { splitSeries: { id: string, name: string, resolvedName: string, mediaCount: number, libraryId: string | null } };

export type ConsoleKindleTargetsQueryVariables = Exact<{ [key: string]: never; }>;


export type ConsoleKindleTargetsQuery = { devices: Array<{ id: string, name: string, kindleEmail: string | null, revokedAt: string | null }> };

export type ConsoleSendToKindleMutationVariables = Exact<{
  mediaId: string | number;
  deviceId: string | number;
}>;


export type ConsoleSendToKindleMutation = { sendToKindle: { deviceId: string, deviceName: string, recipient: string, format: string, bytes: number, converted: boolean, note: string | null } };

export type DeviceFieldsFragment = { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null };

export type DeviceCredentialFieldsFragment = { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string };

export type DeviceEndpointFieldsFragment = { label: string, url: string, username: string | null, secretHint: string };

export type DevicesQueryVariables = Exact<{ [key: string]: never; }>;


export type DevicesQuery = { devices: Array<{ id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }> };

export type CreateDeviceMutationVariables = Exact<{
  kind: DeviceKind;
  name?: string | null | undefined;
}>;


export type CreateDeviceMutation = { createDevice: { device: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string }, endpoints: Array<{ label: string, url: string, username: string | null, secretHint: string }> } };

export type RenameDeviceMutationVariables = Exact<{
  id: string;
  name: string;
}>;


export type RenameDeviceMutation = { renameDevice: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type RotateDeviceCredentialMutationVariables = Exact<{
  id: string;
}>;


export type RotateDeviceCredentialMutation = { rotateDeviceCredential: { device: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null }, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, credentialRef: string, secret: string }, endpoints: Array<{ label: string, url: string, username: string | null, secretHint: string }> } };

export type RevokeDeviceMutationVariables = Exact<{
  id: string;
}>;


export type RevokeDeviceMutation = { revokeDevice: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type SetDeviceTransformProfileMutationVariables = Exact<{
  id: string;
  profile?: unknown;
}>;


export type SetDeviceTransformProfileMutation = { setDeviceTransformProfile: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type SetDeviceLibraryScopeMutationVariables = Exact<{
  id: string;
  libraryIds?: Array<string | number> | string | number | null | undefined;
}>;


export type SetDeviceLibraryScopeMutation = { setDeviceLibraryScope: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type SetDeviceKindleEmailMutationVariables = Exact<{
  id: string;
  email?: string | null | undefined;
}>;


export type SetDeviceKindleEmailMutation = { setDeviceKindleEmail: { id: string, name: string, kind: DeviceKind, transformProfile: unknown, libraryScope: Array<string> | null, kindleEmail: string | null, createdAt: string, lastSeenAt: string | null, lastSyncAt: string | null, lastSyncSummary: unknown, revokedAt: string | null, credential: { kind: DeviceCredentialKind, protocol: DeviceProtocol, secretHint: string } | null } };

export type PendingDevicePairingsQueryVariables = Exact<{ [key: string]: never; }>;


export type PendingDevicePairingsQuery = { pendingDevicePairings: Array<{ id: string, kind: DeviceKind, name: string | null, remoteIp: string, status: DevicePairingStatus, failedAttempts: number, credentialIssued: boolean, createdAt: string, expiresAt: string, approvedAt: string | null }> };

export type ApproveDevicePairingMutationVariables = Exact<{
  pairingId: string | number;
  code?: string | null | undefined;
}>;


export type ApproveDevicePairingMutation = { approveDevicePairing: { id: string, status: DevicePairingStatus } };

export type DenyDevicePairingMutationVariables = Exact<{
  pairingId: string | number;
}>;


export type DenyDevicePairingMutation = { denyDevicePairing: { id: string, status: DevicePairingStatus } };

export type DeviceSeenSubscriptionVariables = Exact<{
  deviceId?: string | null | undefined;
}>;


export type DeviceSeenSubscription = { deviceSeen: { deviceId: string, userId: string, protocol: DeviceProtocol } };

export type ReadingStatsQueryVariables = Exact<{
  span: ReadingStatsSpan;
}>;


export type ReadingStatsQuery = { readingStats: { span: ReadingStatsSpan, from: string | null, to: string, sessions: number, minutes: number, pages: number, booksFinished: number, streakDays: number, days: Array<{ date: string, sessions: number, minutes: number, pages: number }>, devices: Array<{ deviceId: string, name: string | null, kind: DeviceKind | null, sessions: number, minutes: number, pages: number }> } };

export type MyLoginActivityQueryVariables = Exact<{
  userId: string | number;
}>;


export type MyLoginActivityQuery = { loginActivityById: Array<{ id: number, ipAddress: string, userAgent: string, authenticationSuccessful: boolean, timestamp: string }> };

export type ReaderLocatorFieldsFragment = { chapterTitle: string, href: string, title: string | null, type: string, locations: { fragments: Array<string> | null, progression: unknown, position: number | null, totalProgression: unknown, cssSelector: string | null, partialCfi: string | null } | null, text: { before: string | null, highlight: string | null, after: string | null } | null };

export type ReaderBookQueryVariables = Exact<{
  id: string | number;
}>;


export type ReaderBookQuery = { mediaById: { id: string, resolvedName: string, extension: string, pages: number, seriesId: string | null, series: { id: string, name: string }, readProgress: { page: number | null, percentageCompleted: unknown, elapsedSeconds: number, updatedAt: string | null, locator: { chapterTitle: string, href: string, title: string | null, type: string, locations: { fragments: Array<string> | null, progression: unknown, position: number | null, totalProgression: unknown, cssSelector: string | null, partialCfi: string | null } | null, text: { before: string | null, highlight: string | null, after: string | null } | null } | null } | null } | null };

export type ReaderVisiblePagesQueryVariables = Exact<{
  id: string | number;
}>;


export type ReaderVisiblePagesQuery = { mediaVisiblePages: Array<number> };

export type ReaderAnnotationsQueryVariables = Exact<{
  id: string | number;
}>;


export type ReaderAnnotationsQuery = { annotationsByMediaId: Array<{ id: string, annotationText: string | null, createdAt: string, locator: { chapterTitle: string, href: string, title: string | null, type: string, locations: { fragments: Array<string> | null, progression: unknown, position: number | null, totalProgression: unknown, cssSelector: string | null, partialCfi: string | null } | null, text: { before: string | null, highlight: string | null, after: string | null } | null } }> };

export type ReaderUpdateProgressMutationVariables = Exact<{
  id: string | number;
  input: MediaProgressInput;
}>;


export type ReaderUpdateProgressMutation = { updateMediaProgress: { id: number, endPage: number | null, endPercentage: unknown, endLocator: { chapterTitle: string, href: string, title: string | null, type: string, locations: { fragments: Array<string> | null, progression: unknown, position: number | null, totalProgression: unknown, cssSelector: string | null, partialCfi: string | null } | null, text: { before: string | null, highlight: string | null, after: string | null } | null } | null } };

export const ConsoleAnnotationFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleAnnotationFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"AnnotationEntry"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"source"}},{"kind":"Field","name":{"kind":"Name","value":"sourceDeviceId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceDeviceName"}},{"kind":"Field","name":{"kind":"Name","value":"editable"}},{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"fragment"}},{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"excerpt"}},{"kind":"Field","name":{"kind":"Name","value":"note"}},{"kind":"Field","name":{"kind":"Name","value":"color"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"book"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"mediaId"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"authors"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"seriesName"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}}]}}]}}]} as unknown as DocumentNode<ConsoleAnnotationFieldsFragment, unknown>;
export const DashboardBookCardFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DashboardBookCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Media"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"elapsedSeconds"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<DashboardBookCardFragment, unknown>;
export const ConsolePageInfoFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsolePageInfoFragment, unknown>;
export const ConsoleLibraryCardFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleLibraryCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Library"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"lastScannedAt"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"config"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryType"}},{"kind":"Field","name":{"kind":"Name","value":"libraryPattern"}},{"kind":"Field","name":{"kind":"Name","value":"watch"}}]}},{"kind":"Field","name":{"kind":"Name","value":"stats"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"seriesCount"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"completedBooks"}},{"kind":"Field","name":{"kind":"Name","value":"inProgressBooks"}},{"kind":"Field","name":{"kind":"Name","value":"totalBytes"}}]}}]}}]} as unknown as DocumentNode<ConsoleLibraryCardFragment, unknown>;
export const ConsoleSeriesCardFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleSeriesCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Series"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}},{"kind":"Field","name":{"kind":"Name","value":"readCount"}},{"kind":"Field","name":{"kind":"Name","value":"unreadCount"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"isComplete"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}}]}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<ConsoleSeriesCardFragment, unknown>;
export const ConsoleBookRowFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleBookRow"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Media"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"size"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}},{"kind":"Field","name":{"kind":"Name","value":"writers"}}]}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readHistory"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"readthroughNumber"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"dnf"}}]}}]}}]} as unknown as DocumentNode<ConsoleBookRowFragment, unknown>;
export const DeviceFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<DeviceFieldsFragment, unknown>;
export const DeviceCredentialFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}}]} as unknown as DocumentNode<DeviceCredentialFieldsFragment, unknown>;
export const DeviceEndpointFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<DeviceEndpointFieldsFragment, unknown>;
export const ReaderLocatorFieldsFragmentDoc = {"kind":"Document","definitions":[{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ReaderLocatorFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"ReadiumLocator"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"type"}},{"kind":"Field","name":{"kind":"Name","value":"locations"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"fragments"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"position"}},{"kind":"Field","name":{"kind":"Name","value":"totalProgression"}},{"kind":"Field","name":{"kind":"Name","value":"cssSelector"}},{"kind":"Field","name":{"kind":"Name","value":"partialCfi"}}]}},{"kind":"Field","name":{"kind":"Name","value":"text"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"before"}},{"kind":"Field","name":{"kind":"Name","value":"highlight"}},{"kind":"Field","name":{"kind":"Name","value":"after"}}]}}]}}]} as unknown as DocumentNode<ReaderLocatorFieldsFragment, unknown>;
export const ConsoleAnnotationsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleAnnotations"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"AnnotationFilterInput"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"annotations"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"total"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"hasNext"}},{"kind":"Field","name":{"kind":"Name","value":"items"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleAnnotationFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleAnnotationFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"AnnotationEntry"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"source"}},{"kind":"Field","name":{"kind":"Name","value":"sourceDeviceId"}},{"kind":"Field","name":{"kind":"Name","value":"sourceDeviceName"}},{"kind":"Field","name":{"kind":"Name","value":"editable"}},{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"fragment"}},{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"excerpt"}},{"kind":"Field","name":{"kind":"Name","value":"note"}},{"kind":"Field","name":{"kind":"Name","value":"color"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"book"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"mediaId"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"authors"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"seriesName"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}}]}}]}}]} as unknown as DocumentNode<ConsoleAnnotationsQuery, ConsoleAnnotationsQueryVariables>;
export const ConsoleAnnotationBooksDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleAnnotationBooks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"annotations"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"page"},"value":{"kind":"IntValue","value":"1"}},{"kind":"ObjectField","name":{"kind":"Name","value":"pageSize"},"value":{"kind":"IntValue","value":"500"}}]}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"items"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"book"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"mediaId"}},{"kind":"Field","name":{"kind":"Name","value":"title"}}]}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleAnnotationBooksQuery, ConsoleAnnotationBooksQueryVariables>;
export const ConsoleUpdateAnnotationDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleUpdateAnnotation"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"UpdateAnnotationInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"updateAnnotation"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"annotationText"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<ConsoleUpdateAnnotationMutation, ConsoleUpdateAnnotationMutationVariables>;
export const ConsoleDeleteAnnotationDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleDeleteAnnotation"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deleteAnnotation"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}}]}}]}}]} as unknown as DocumentNode<ConsoleDeleteAnnotationMutation, ConsoleDeleteAnnotationMutationVariables>;
export const ConsoleAnnotationSinksDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleAnnotationSinks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"annotationSinks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"annotationSyncStatus"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"userId"}},{"kind":"Field","name":{"kind":"Name","value":"pending"}},{"kind":"Field","name":{"kind":"Name","value":"sinks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"sinkId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"lastRunAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastError"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleAnnotationSinksQuery, ConsoleAnnotationSinksQueryVariables>;
export const ConsoleSetAnnotationSinkSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleSetAnnotationSinkSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"sinkId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"settings"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"JSON"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"enabled"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"Boolean"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setAnnotationSinkSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"sinkId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"sinkId"}}},{"kind":"Argument","name":{"kind":"Name","value":"settings"},"value":{"kind":"Variable","name":{"kind":"Name","value":"settings"}}},{"kind":"Argument","name":{"kind":"Name","value":"enabled"},"value":{"kind":"Variable","name":{"kind":"Name","value":"enabled"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"userId"}},{"kind":"Field","name":{"kind":"Name","value":"pending"}},{"kind":"Field","name":{"kind":"Name","value":"sinks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"sinkId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"lastRunAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastError"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleSetAnnotationSinkSettingsMutation, ConsoleSetAnnotationSinkSettingsMutationVariables>;
export const ConsoleRunAnnotationSyncDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleRunAnnotationSync"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"runAnnotationSync"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"userId"}},{"kind":"Field","name":{"kind":"Name","value":"pending"}},{"kind":"Field","name":{"kind":"Name","value":"sinks"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"sinkId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}},{"kind":"Field","name":{"kind":"Name","value":"lastRunAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastError"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleRunAnnotationSyncMutation, ConsoleRunAnnotationSyncMutationVariables>;
export const DashboardViewerDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"DashboardViewer"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"me"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"isServerOwner"}},{"kind":"Field","name":{"kind":"Name","value":"permissions"}}]}}]}}]} as unknown as DocumentNode<DashboardViewerQuery, DashboardViewerQueryVariables>;
export const DashboardKeepReadingDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"DashboardKeepReading"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"keepReading"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DashboardBookCard"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DashboardBookCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Media"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"elapsedSeconds"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<DashboardKeepReadingQuery, DashboardKeepReadingQueryVariables>;
export const DashboardRecentlyAddedDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"DashboardRecentlyAdded"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"recentlyAddedMedia"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DashboardBookCard"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DashboardBookCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Media"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"elapsedSeconds"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}}]}}]} as unknown as DocumentNode<DashboardRecentlyAddedQuery, DashboardRecentlyAddedQueryVariables>;
export const DashboardJobsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"DashboardJobs"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"jobs"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"msElapsed"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}}]}}]}}]}}]} as unknown as DocumentNode<DashboardJobsQuery, DashboardJobsQueryVariables>;
export const DashboardLiveEventsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"subscription","name":{"kind":"Name","value":"DashboardLiveEvents"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"readEvents"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"__typename"}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"JobStarted"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"JobUpdate"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"message"}},{"kind":"Field","name":{"kind":"Name","value":"subtitle"}},{"kind":"Field","name":{"kind":"Name","value":"completedTasks"}},{"kind":"Field","name":{"kind":"Name","value":"remainingTasks"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"JobOutput"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceSeen"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}}]}},{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"JobQueueStatus"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"count"}},{"kind":"Field","name":{"kind":"Name","value":"countByType"}}]}}]}}]}}]} as unknown as DocumentNode<DashboardLiveEventsSubscription, DashboardLiveEventsSubscriptionVariables>;
export const NotificationSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"NotificationSettings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"notificationChannels"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"settings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"key"}},{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"valueType"}},{"kind":"Field","name":{"kind":"Name","value":"required"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}},{"kind":"Field","name":{"kind":"Name","value":"defaultValue"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"helpUrl"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"notificationRules"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"eventKind"}},{"kind":"Field","name":{"kind":"Name","value":"channelId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}}]}},{"kind":"Field","name":{"kind":"Name","value":"notificationChannelSettings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"channelId"}},{"kind":"Field","name":{"kind":"Name","value":"values"}}]}}]}}]} as unknown as DocumentNode<NotificationSettingsQuery, NotificationSettingsQueryVariables>;
export const SetNotificationRuleDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetNotificationRule"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"eventKind"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"NotificationKind"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"channelId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"enabled"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Boolean"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setNotificationRule"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"eventKind"},"value":{"kind":"Variable","name":{"kind":"Name","value":"eventKind"}}},{"kind":"Argument","name":{"kind":"Name","value":"channelId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"channelId"}}},{"kind":"Argument","name":{"kind":"Name","value":"enabled"},"value":{"kind":"Variable","name":{"kind":"Name","value":"enabled"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"eventKind"}},{"kind":"Field","name":{"kind":"Name","value":"channelId"}},{"kind":"Field","name":{"kind":"Name","value":"enabled"}}]}}]}}]} as unknown as DocumentNode<SetNotificationRuleMutation, SetNotificationRuleMutationVariables>;
export const SetNotificationChannelSettingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetNotificationChannelSettings"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SetNotificationChannelSettingsInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setNotificationChannelSettings"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"channelId"}},{"kind":"Field","name":{"kind":"Name","value":"values"}}]}}]}}]} as unknown as DocumentNode<SetNotificationChannelSettingsMutation, SetNotificationChannelSettingsMutationVariables>;
export const TestNotificationChannelDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"TestNotificationChannel"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"channelId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"testNotificationChannel"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"channelId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"channelId"}}}]}]}}]} as unknown as DocumentNode<TestNotificationChannelMutation, TestNotificationChannelMutationVariables>;
export const ConsoleLibrariesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleLibraries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}},{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"ListValue","values":[{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"field"},"value":{"kind":"EnumValue","value":"NAME"}},{"kind":"ObjectField","name":{"kind":"Name","value":"direction"},"value":{"kind":"EnumValue","value":"ASC"}}]}]}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleLibraryCard"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsolePageInfo"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleLibraryCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Library"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"lastScannedAt"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"config"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryType"}},{"kind":"Field","name":{"kind":"Name","value":"libraryPattern"}},{"kind":"Field","name":{"kind":"Name","value":"watch"}}]}},{"kind":"Field","name":{"kind":"Name","value":"stats"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"seriesCount"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"completedBooks"}},{"kind":"Field","name":{"kind":"Name","value":"inProgressBooks"}},{"kind":"Field","name":{"kind":"Name","value":"totalBytes"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsoleLibrariesQuery, ConsoleLibrariesQueryVariables>;
export const ConsoleLibraryOptionsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleLibraryOptions"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"offset"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"page"},"value":{"kind":"IntValue","value":"1"}},{"kind":"ObjectField","name":{"kind":"Name","value":"pageSize"},"value":{"kind":"IntValue","value":"100"}}]}}]}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleLibraryOptionsQuery, ConsoleLibraryOptionsQueryVariables>;
export const ConsoleLibraryDetailDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleLibraryDetail"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleLibraryCard"}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleLibraryCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Library"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"lastScannedAt"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"config"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryType"}},{"kind":"Field","name":{"kind":"Name","value":"libraryPattern"}},{"kind":"Field","name":{"kind":"Name","value":"watch"}}]}},{"kind":"Field","name":{"kind":"Name","value":"stats"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"seriesCount"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"completedBooks"}},{"kind":"Field","name":{"kind":"Name","value":"inProgressBooks"}},{"kind":"Field","name":{"kind":"Name","value":"totalBytes"}}]}}]}}]} as unknown as DocumentNode<ConsoleLibraryDetailQuery, ConsoleLibraryDetailQueryVariables>;
export const ConsoleLibrarySeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleLibrarySeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SeriesFilterInput"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"orderBy"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SeriesOrderBy"}}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"series"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"Variable","name":{"kind":"Name","value":"orderBy"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleSeriesCard"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsolePageInfo"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleSeriesCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Series"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}},{"kind":"Field","name":{"kind":"Name","value":"readCount"}},{"kind":"Field","name":{"kind":"Name","value":"unreadCount"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"isComplete"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}}]}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsoleLibrarySeriesQuery, ConsoleLibrarySeriesQueryVariables>;
export const ConsoleBooksDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleBooks"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"MediaFilterInput"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"orderBy"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"MediaOrderBy"}}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"media"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"Variable","name":{"kind":"Name","value":"orderBy"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleBookRow"}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsolePageInfo"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleBookRow"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Media"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"size"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}}]}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}},{"kind":"Field","name":{"kind":"Name","value":"writers"}}]}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readHistory"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"readthroughNumber"}},{"kind":"Field","name":{"kind":"Name","value":"completedAt"}},{"kind":"Field","name":{"kind":"Name","value":"dnf"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsoleBooksQuery, ConsoleBooksQueryVariables>;
export const ConsoleSeriesDetailDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleSeriesDetail"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"seriesById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleSeriesCard"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedDescription"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}},{"kind":"Field","name":{"kind":"Name","value":"library"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"config"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryType"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"stats"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"completedBooks"}},{"kind":"Field","name":{"kind":"Name","value":"inProgressBooks"}},{"kind":"Field","name":{"kind":"Name","value":"totalReadingTimeSeconds"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"ageRating"}},{"kind":"Field","name":{"kind":"Name","value":"booktype"}},{"kind":"Field","name":{"kind":"Name","value":"characters"}},{"kind":"Field","name":{"kind":"Name","value":"collects"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"series"}},{"kind":"Field","name":{"kind":"Name","value":"comicid"}},{"kind":"Field","name":{"kind":"Name","value":"issueid"}},{"kind":"Field","name":{"kind":"Name","value":"issues"}}]}},{"kind":"Field","name":{"kind":"Name","value":"comicImage"}},{"kind":"Field","name":{"kind":"Name","value":"comicid"}},{"kind":"Field","name":{"kind":"Name","value":"descriptionFormatted"}},{"kind":"Field","name":{"kind":"Name","value":"genres"}},{"kind":"Field","name":{"kind":"Name","value":"imprint"}},{"kind":"Field","name":{"kind":"Name","value":"links"}},{"kind":"Field","name":{"kind":"Name","value":"metaType"}},{"kind":"Field","name":{"kind":"Name","value":"publicationRun"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"summary"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"totalIssues"}},{"kind":"Field","name":{"kind":"Name","value":"volume"}},{"kind":"Field","name":{"kind":"Name","value":"writers"}},{"kind":"Field","name":{"kind":"Name","value":"year"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleSeriesCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Series"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}},{"kind":"Field","name":{"kind":"Name","value":"readCount"}},{"kind":"Field","name":{"kind":"Name","value":"unreadCount"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"isComplete"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"thumbnail"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"url"}}]}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"publisher"}}]}},{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}}]}}]} as unknown as DocumentNode<ConsoleSeriesDetailQuery, ConsoleSeriesDetailQueryVariables>;
export const ConsoleSeriesPickerDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleSeriesPicker"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SeriesFilterInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"series"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"orderBy"},"value":{"kind":"ListValue","values":[{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"series"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"field"},"value":{"kind":"EnumValue","value":"NAME"}},{"kind":"ObjectField","name":{"kind":"Name","value":"direction"},"value":{"kind":"EnumValue","value":"ASC"}}]}}]}]}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"offset"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"page"},"value":{"kind":"IntValue","value":"1"}},{"kind":"ObjectField","name":{"kind":"Name","value":"pageSize"},"value":{"kind":"IntValue","value":"200"}}]}}]}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleSeriesPickerQuery, ConsoleSeriesPickerQueryVariables>;
export const ConsoleAuthorsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleAuthors"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"search"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"Pagination"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"authors"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"search"},"value":{"kind":"Variable","name":{"kind":"Name","value":"search"}}},{"kind":"Argument","name":{"kind":"Name","value":"libraryId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryId"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pagination"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"nodes"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"books"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}}]}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}}]}}]}},{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsolePageInfo"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsoleAuthorsQuery, ConsoleAuthorsQueryVariables>;
export const ConsoleLibraryPublishersDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleLibraryPublishers"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"publishers"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"sort"},"value":{"kind":"EnumValue","value":"ASC"}}]}]}}]}}]} as unknown as DocumentNode<ConsoleLibraryPublishersQuery, ConsoleLibraryPublishersQueryVariables>;
export const ConsoleTagsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleTags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"tags"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}}]}}]}}]} as unknown as DocumentNode<ConsoleTagsQuery, ConsoleTagsQueryVariables>;
export const ConsoleEntityBookCountDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleEntityBookCount"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"filter"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"MediaFilterInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"media"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"filter"},"value":{"kind":"Variable","name":{"kind":"Name","value":"filter"}}},{"kind":"Argument","name":{"kind":"Name","value":"pagination"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"offset"},"value":{"kind":"ObjectValue","fields":[{"kind":"ObjectField","name":{"kind":"Name","value":"page"},"value":{"kind":"IntValue","value":"1"}},{"kind":"ObjectField","name":{"kind":"Name","value":"pageSize"},"value":{"kind":"IntValue","value":"1"}}]}}]}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"pageInfo"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsolePageInfo"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsolePageInfo"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"PaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"InlineFragment","typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"OffsetPaginationInfo"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"totalItems"}},{"kind":"Field","name":{"kind":"Name","value":"totalPages"}},{"kind":"Field","name":{"kind":"Name","value":"currentPage"}},{"kind":"Field","name":{"kind":"Name","value":"pageSize"}}]}}]}}]} as unknown as DocumentNode<ConsoleEntityBookCountQuery, ConsoleEntityBookCountQueryVariables>;
export const ConsoleCreateLibraryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleCreateLibrary"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"CreateOrUpdateLibraryInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"createLibrary"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ConsoleLibraryCard"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ConsoleLibraryCard"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Library"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"description"}},{"kind":"Field","name":{"kind":"Name","value":"path"}},{"kind":"Field","name":{"kind":"Name","value":"emoji"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"lastScannedAt"}},{"kind":"Field","name":{"kind":"Name","value":"sourceProvider"}},{"kind":"Field","name":{"kind":"Name","value":"config"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"libraryType"}},{"kind":"Field","name":{"kind":"Name","value":"libraryPattern"}},{"kind":"Field","name":{"kind":"Name","value":"watch"}}]}},{"kind":"Field","name":{"kind":"Name","value":"stats"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"seriesCount"}},{"kind":"Field","name":{"kind":"Name","value":"bookCount"}},{"kind":"Field","name":{"kind":"Name","value":"completedBooks"}},{"kind":"Field","name":{"kind":"Name","value":"inProgressBooks"}},{"kind":"Field","name":{"kind":"Name","value":"totalBytes"}}]}}]}}]} as unknown as DocumentNode<ConsoleCreateLibraryMutation, ConsoleCreateLibraryMutationVariables>;
export const ConsoleScanLibraryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleScanLibrary"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"scanLibrary"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleScanLibraryMutation, ConsoleScanLibraryMutationVariables>;
export const ConsoleAnalyzeLibraryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleAnalyzeLibrary"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"analyzeLibrary"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleAnalyzeLibraryMutation, ConsoleAnalyzeLibraryMutationVariables>;
export const ConsoleFinishMediaDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleFinishMedia"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"finishMediaProgress"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleFinishMediaMutation, ConsoleFinishMediaMutationVariables>;
export const ConsoleResetMediaProgressDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleResetMediaProgress"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"clearMediaProgress"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]},{"kind":"Field","name":{"kind":"Name","value":"deleteMediaReadingHistory"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleResetMediaProgressMutation, ConsoleResetMediaProgressMutationVariables>;
export const ConsoleFinishSeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleFinishSeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"finishSeriesProgress"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleFinishSeriesMutation, ConsoleFinishSeriesMutationVariables>;
export const ConsoleClearSeriesHistoryDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleClearSeriesHistory"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"clearSeriesReadingHistory"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ConsoleClearSeriesHistoryMutation, ConsoleClearSeriesHistoryMutationVariables>;
export const ConsoleRenameSeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleRenameSeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"SeriesMetadataInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"updateSeriesMetadata"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"metadata"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"title"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleRenameSeriesMutation, ConsoleRenameSeriesMutationVariables>;
export const ConsoleMoveMediaToSeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleMoveMediaToSeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"seriesId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"moveMediaToSeries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaIds"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}}},{"kind":"Argument","name":{"kind":"Name","value":"seriesId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"seriesId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}}]}}]}}]} as unknown as DocumentNode<ConsoleMoveMediaToSeriesMutation, ConsoleMoveMediaToSeriesMutationVariables>;
export const ConsoleMergeSeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleMergeSeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"keep"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"drop"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"mergeSeries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"keep"},"value":{"kind":"Variable","name":{"kind":"Name","value":"keep"}}},{"kind":"Argument","name":{"kind":"Name","value":"drop"},"value":{"kind":"Variable","name":{"kind":"Name","value":"drop"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"droppedSeriesId"}},{"kind":"Field","name":{"kind":"Name","value":"moved"}},{"kind":"Field","name":{"kind":"Name","value":"missingFiles"}},{"kind":"Field","name":{"kind":"Name","value":"droppedDirectory"}},{"kind":"Field","name":{"kind":"Name","value":"kept"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}}]}}]}}]}}]} as unknown as DocumentNode<ConsoleMergeSeriesMutation, ConsoleMergeSeriesMutationVariables>;
export const ConsoleSplitSeriesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleSplitSeries"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}},"type":{"kind":"NonNullType","type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"name"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"splitSeries"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaIds"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaIds"}}},{"kind":"Argument","name":{"kind":"Name","value":"name"},"value":{"kind":"Variable","name":{"kind":"Name","value":"name"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"mediaCount"}},{"kind":"Field","name":{"kind":"Name","value":"libraryId"}}]}}]}}]} as unknown as DocumentNode<ConsoleSplitSeriesMutation, ConsoleSplitSeriesMutationVariables>;
export const ConsoleKindleTargetsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ConsoleKindleTargets"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}}]}}]}}]} as unknown as DocumentNode<ConsoleKindleTargetsQuery, ConsoleKindleTargetsQueryVariables>;
export const ConsoleSendToKindleDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ConsoleSendToKindle"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"sendToKindle"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"mediaId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"mediaId"}}},{"kind":"Argument","name":{"kind":"Name","value":"deviceId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"deviceName"}},{"kind":"Field","name":{"kind":"Name","value":"recipient"}},{"kind":"Field","name":{"kind":"Name","value":"format"}},{"kind":"Field","name":{"kind":"Name","value":"bytes"}},{"kind":"Field","name":{"kind":"Name","value":"converted"}},{"kind":"Field","name":{"kind":"Name","value":"note"}}]}}]}}]} as unknown as DocumentNode<ConsoleSendToKindleMutation, ConsoleSendToKindleMutationVariables>;
export const DevicesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"Devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<DevicesQuery, DevicesQueryVariables>;
export const CreateDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"CreateDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"kind"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceKind"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"name"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"createDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"kind"},"value":{"kind":"Variable","name":{"kind":"Name","value":"kind"}}},{"kind":"Argument","name":{"kind":"Name","value":"name"},"value":{"kind":"Variable","name":{"kind":"Name","value":"name"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"device"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceCredentialFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"endpoints"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceEndpointFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<CreateDeviceMutation, CreateDeviceMutationVariables>;
export const RenameDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RenameDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"name"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"renameDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"name"},"value":{"kind":"Variable","name":{"kind":"Name","value":"name"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<RenameDeviceMutation, RenameDeviceMutationVariables>;
export const RotateDeviceCredentialDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RotateDeviceCredential"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"rotateDeviceCredential"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"device"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceCredentialFields"}}]}},{"kind":"Field","name":{"kind":"Name","value":"endpoints"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceEndpointFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceCredentialFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"IssuedDeviceCredential"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"credentialRef"}},{"kind":"Field","name":{"kind":"Name","value":"secret"}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceEndpointFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"DeviceEndpoint"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"label"}},{"kind":"Field","name":{"kind":"Name","value":"url"}},{"kind":"Field","name":{"kind":"Name","value":"username"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]} as unknown as DocumentNode<RotateDeviceCredentialMutation, RotateDeviceCredentialMutationVariables>;
export const RevokeDeviceDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"RevokeDevice"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"revokeDevice"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<RevokeDeviceMutation, RevokeDeviceMutationVariables>;
export const SetDeviceTransformProfileDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetDeviceTransformProfile"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"profile"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"JSON"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setDeviceTransformProfile"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"profile"},"value":{"kind":"Variable","name":{"kind":"Name","value":"profile"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<SetDeviceTransformProfileMutation, SetDeviceTransformProfileMutationVariables>;
export const SetDeviceLibraryScopeDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetDeviceLibraryScope"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"libraryIds"}},"type":{"kind":"ListType","type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setDeviceLibraryScope"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"libraryIds"},"value":{"kind":"Variable","name":{"kind":"Name","value":"libraryIds"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<SetDeviceLibraryScopeMutation, SetDeviceLibraryScopeMutationVariables>;
export const SetDeviceKindleEmailDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"SetDeviceKindleEmail"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"email"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"setDeviceKindleEmail"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"email"},"value":{"kind":"Variable","name":{"kind":"Name","value":"email"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"DeviceFields"}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"DeviceFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"Device"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"transformProfile"}},{"kind":"Field","name":{"kind":"Name","value":"libraryScope"}},{"kind":"Field","name":{"kind":"Name","value":"kindleEmail"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSeenAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncAt"}},{"kind":"Field","name":{"kind":"Name","value":"lastSyncSummary"}},{"kind":"Field","name":{"kind":"Name","value":"revokedAt"}},{"kind":"Field","name":{"kind":"Name","value":"credential"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}},{"kind":"Field","name":{"kind":"Name","value":"secretHint"}}]}}]}}]} as unknown as DocumentNode<SetDeviceKindleEmailMutation, SetDeviceKindleEmailMutationVariables>;
export const PendingDevicePairingsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"PendingDevicePairings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"pendingDevicePairings"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"remoteIp"}},{"kind":"Field","name":{"kind":"Name","value":"status"}},{"kind":"Field","name":{"kind":"Name","value":"failedAttempts"}},{"kind":"Field","name":{"kind":"Name","value":"credentialIssued"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"expiresAt"}},{"kind":"Field","name":{"kind":"Name","value":"approvedAt"}}]}}]}}]} as unknown as DocumentNode<PendingDevicePairingsQuery, PendingDevicePairingsQueryVariables>;
export const ApproveDevicePairingDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ApproveDevicePairing"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"code"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"approveDevicePairing"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pairingId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}}},{"kind":"Argument","name":{"kind":"Name","value":"code"},"value":{"kind":"Variable","name":{"kind":"Name","value":"code"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}}]}}]}}]} as unknown as DocumentNode<ApproveDevicePairingMutation, ApproveDevicePairingMutationVariables>;
export const DenyDevicePairingDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"DenyDevicePairing"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"denyDevicePairing"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"pairingId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"pairingId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"status"}}]}}]}}]} as unknown as DocumentNode<DenyDevicePairingMutation, DenyDevicePairingMutationVariables>;
export const DeviceSeenDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"subscription","name":{"kind":"Name","value":"DeviceSeen"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}},"type":{"kind":"NamedType","name":{"kind":"Name","value":"String"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceSeen"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"deviceId"},"value":{"kind":"Variable","name":{"kind":"Name","value":"deviceId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"userId"}},{"kind":"Field","name":{"kind":"Name","value":"protocol"}}]}}]}}]} as unknown as DocumentNode<DeviceSeenSubscription, DeviceSeenSubscriptionVariables>;
export const ReadingStatsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ReadingStats"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"span"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ReadingStatsSpan"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"readingStats"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"span"},"value":{"kind":"Variable","name":{"kind":"Name","value":"span"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"span"}},{"kind":"Field","name":{"kind":"Name","value":"from"}},{"kind":"Field","name":{"kind":"Name","value":"to"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"booksFinished"}},{"kind":"Field","name":{"kind":"Name","value":"streakDays"}},{"kind":"Field","name":{"kind":"Name","value":"days"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"date"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}}]}},{"kind":"Field","name":{"kind":"Name","value":"devices"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"deviceId"}},{"kind":"Field","name":{"kind":"Name","value":"name"}},{"kind":"Field","name":{"kind":"Name","value":"kind"}},{"kind":"Field","name":{"kind":"Name","value":"sessions"}},{"kind":"Field","name":{"kind":"Name","value":"minutes"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}}]}}]}}]}}]} as unknown as DocumentNode<ReadingStatsQuery, ReadingStatsQueryVariables>;
export const MyLoginActivityDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"MyLoginActivity"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"userId"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"loginActivityById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"userId"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"ipAddress"}},{"kind":"Field","name":{"kind":"Name","value":"userAgent"}},{"kind":"Field","name":{"kind":"Name","value":"authenticationSuccessful"}},{"kind":"Field","name":{"kind":"Name","value":"timestamp"}}]}}]}}]} as unknown as DocumentNode<MyLoginActivityQuery, MyLoginActivityQueryVariables>;
export const ReaderBookDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ReaderBook"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"mediaById"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"resolvedName"}},{"kind":"Field","name":{"kind":"Name","value":"extension"}},{"kind":"Field","name":{"kind":"Name","value":"pages"}},{"kind":"Field","name":{"kind":"Name","value":"seriesId"}},{"kind":"Field","name":{"kind":"Name","value":"series"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"name"}}]}},{"kind":"Field","name":{"kind":"Name","value":"readProgress"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"page"}},{"kind":"Field","name":{"kind":"Name","value":"percentageCompleted"}},{"kind":"Field","name":{"kind":"Name","value":"elapsedSeconds"}},{"kind":"Field","name":{"kind":"Name","value":"updatedAt"}},{"kind":"Field","name":{"kind":"Name","value":"locator"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ReaderLocatorFields"}}]}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ReaderLocatorFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"ReadiumLocator"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"type"}},{"kind":"Field","name":{"kind":"Name","value":"locations"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"fragments"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"position"}},{"kind":"Field","name":{"kind":"Name","value":"totalProgression"}},{"kind":"Field","name":{"kind":"Name","value":"cssSelector"}},{"kind":"Field","name":{"kind":"Name","value":"partialCfi"}}]}},{"kind":"Field","name":{"kind":"Name","value":"text"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"before"}},{"kind":"Field","name":{"kind":"Name","value":"highlight"}},{"kind":"Field","name":{"kind":"Name","value":"after"}}]}}]}}]} as unknown as DocumentNode<ReaderBookQuery, ReaderBookQueryVariables>;
export const ReaderVisiblePagesDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ReaderVisiblePages"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"mediaVisiblePages"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}]}]}}]} as unknown as DocumentNode<ReaderVisiblePagesQuery, ReaderVisiblePagesQueryVariables>;
export const ReaderAnnotationsDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"query","name":{"kind":"Name","value":"ReaderAnnotations"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"annotationsByMediaId"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"annotationText"}},{"kind":"Field","name":{"kind":"Name","value":"createdAt"}},{"kind":"Field","name":{"kind":"Name","value":"locator"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ReaderLocatorFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ReaderLocatorFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"ReadiumLocator"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"type"}},{"kind":"Field","name":{"kind":"Name","value":"locations"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"fragments"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"position"}},{"kind":"Field","name":{"kind":"Name","value":"totalProgression"}},{"kind":"Field","name":{"kind":"Name","value":"cssSelector"}},{"kind":"Field","name":{"kind":"Name","value":"partialCfi"}}]}},{"kind":"Field","name":{"kind":"Name","value":"text"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"before"}},{"kind":"Field","name":{"kind":"Name","value":"highlight"}},{"kind":"Field","name":{"kind":"Name","value":"after"}}]}}]}}]} as unknown as DocumentNode<ReaderAnnotationsQuery, ReaderAnnotationsQueryVariables>;
export const ReaderUpdateProgressDocument = {"kind":"Document","definitions":[{"kind":"OperationDefinition","operation":"mutation","name":{"kind":"Name","value":"ReaderUpdateProgress"},"variableDefinitions":[{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"id"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"ID"}}}},{"kind":"VariableDefinition","variable":{"kind":"Variable","name":{"kind":"Name","value":"input"}},"type":{"kind":"NonNullType","type":{"kind":"NamedType","name":{"kind":"Name","value":"MediaProgressInput"}}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"updateMediaProgress"},"arguments":[{"kind":"Argument","name":{"kind":"Name","value":"id"},"value":{"kind":"Variable","name":{"kind":"Name","value":"id"}}},{"kind":"Argument","name":{"kind":"Name","value":"input"},"value":{"kind":"Variable","name":{"kind":"Name","value":"input"}}}],"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"id"}},{"kind":"Field","name":{"kind":"Name","value":"endPage"}},{"kind":"Field","name":{"kind":"Name","value":"endPercentage"}},{"kind":"Field","name":{"kind":"Name","value":"endLocator"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"FragmentSpread","name":{"kind":"Name","value":"ReaderLocatorFields"}}]}}]}}]}},{"kind":"FragmentDefinition","name":{"kind":"Name","value":"ReaderLocatorFields"},"typeCondition":{"kind":"NamedType","name":{"kind":"Name","value":"ReadiumLocator"}},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"chapterTitle"}},{"kind":"Field","name":{"kind":"Name","value":"href"}},{"kind":"Field","name":{"kind":"Name","value":"title"}},{"kind":"Field","name":{"kind":"Name","value":"type"}},{"kind":"Field","name":{"kind":"Name","value":"locations"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"fragments"}},{"kind":"Field","name":{"kind":"Name","value":"progression"}},{"kind":"Field","name":{"kind":"Name","value":"position"}},{"kind":"Field","name":{"kind":"Name","value":"totalProgression"}},{"kind":"Field","name":{"kind":"Name","value":"cssSelector"}},{"kind":"Field","name":{"kind":"Name","value":"partialCfi"}}]}},{"kind":"Field","name":{"kind":"Name","value":"text"},"selectionSet":{"kind":"SelectionSet","selections":[{"kind":"Field","name":{"kind":"Name","value":"before"}},{"kind":"Field","name":{"kind":"Name","value":"highlight"}},{"kind":"Field","name":{"kind":"Name","value":"after"}}]}}]}}]} as unknown as DocumentNode<ReaderUpdateProgressMutation, ReaderUpdateProgressMutationVariables>;