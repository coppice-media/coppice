/* eslint-disable */
/** Internal type. DO NOT USE DIRECTLY. */
type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] }
/** Internal type. DO NOT USE DIRECTLY. */
export type Incremental<T> =
	| T
	| { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never }
import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core'
/** Granularity of cues in a sync map or alignment request. */
export type AlignGranularity = 'SENTENCE' | 'WORD'

/**
 * Narrows the cross-book annotation hub. Every field is a conjunction; an
 * empty list is the same as omitting it.
 */
export type AnnotationFilterInput = {
	kind?: Array<AnnotationKind> | null | undefined
	/** Every book of one library */
	libraryId?: string | number | null | undefined
	/** One book, by its Stump media id */
	mediaId?: string | number | null | undefined
	/**
	 * Case-insensitive substring match over the selected passage, the note,
	 * and the book title
	 */
	query?: string | null | undefined
	/** Every book of one series */
	seriesId?: string | number | null | undefined
	/** Only annotations created at or after this instant */
	since?: string | null | undefined
	/**
	 * Where the annotation came from, as reported by
	 * [`AnnotationEntry::source`](crate::object::annotation::AnnotationEntry)
	 */
	source?: Array<DeviceKind> | null | undefined
	/**
	 * Durable registered device id that pushed the annotation. This is
	 * independent of whether the device's current credential is active, so
	 * revoked-device history remains filterable.
	 */
	sourceDeviceId?: string | number | null | undefined
}

/**
 * What an annotation is.
 *
 * A native `media_annotations` row always carries a Readium locator, so the
 * kind follows the locator's `text.highlight`: a row with that selected
 * passage is a `HIGHLIGHT`, a row carrying only the user's text is a `NOTE`.
 * Native `bookmarks` rows and liseur `bookmark` records are `BOOKMARK`.
 */
export type AnnotationKind = 'BOOKMARK' | 'HIGHLIGHT' | 'NOTE'

/**
 * How an audiobook's chapter marks were obtained.
 *
 * This is provenance, never a preference: a list synthesized one-chapter-
 * per-file ([`Self::PerTrack`]) must not be presented as if the publisher
 * shipped it, and a client that wants to hide synthetic chapters can only do
 * that if the mechanism survives the probe. The variants name the concrete
 * container mechanism rather than a quality tier so a new container format
 * adds a variant instead of silently widening an existing one.
 */
export type AudioChapterSource =
	/** ID3v2 `CHAP`/`CTOC` frames. */
	| 'ID_3_CHAP'
	/**
	 * A QuickTime text chapter track, linked from the audio track by a
	 * `tref`/`chap` reference.
	 */
	| 'MP_4_CHAPTER_TRACK'
	/** The Nero `chpl` atom in `moov/udta` of an MP4/M4B container. */
	| 'MP_4_CHPL'
	/** The publication has no chapters. */
	| 'NONE'
	/**
	 * Synthesized: one chapter per file of a folder audiobook. The publisher
	 * shipped no chapter marks at all.
	 */
	| 'PER_TRACK'
	/** `CHAPTERxxx`/`CHAPTERxxxNAME` Vorbis comments (Ogg, Opus, FLAC). */
	| 'VORBIS_COMMENT'

/**
 * A listening position: milliseconds from the start of the publication,
 * plus the file the client was in for a multi-file audiobook. A recording
 * has no pages, so this carries neither a page nor a locator.
 */
export type AudioProgressInput = {
	deviceId?: string | null | undefined
	elapsedSecondsDelta?: number | null | undefined
	isComplete?: boolean | null | undefined
	positionMs: number
	resetElapsedSeconds?: boolean | null | undefined
	/** The 0-based `mediaAudioTracks.index` the position fell in. */
	trackIndex?: number | null | undefined
}

export type BookClubInvitationInput = {
	role?: BookClubMemberRole | null | undefined
	userId: string
}

export type BookClubInvitationResponseInput = {
	accept: boolean
	member?: BookClubMemberInput | null | undefined
}

export type BookClubMemberInput = {
	displayName?: string | null | undefined
	userId: string
}

/** The role of a member within a book club */
export type BookClubMemberRole = 'ADMIN' | 'CREATOR' | 'MEMBER' | 'MODERATOR'

/** Distinct lanes in a merged work view. */
export type BookEditionKind = 'AUDIOBOOK' | 'EBOOK' | 'OTHER'

export type BookMetadataApplyInput = {
	metadata: MediaMetadataInput
	scope: BookMetadataScope
	selectedFields: Array<MetadataField>
}

/** The explicit metadata destination selected by a book-detail operation. */
export type BookMetadataScope = 'AUDIOBOOK' | 'BOTH' | 'EBOOK' | 'WORK'

/** Cache/readiness states for an accepted read-aloud artifact. */
export type BookReadAloudStatus =
	| 'CACHE_MISSING'
	| 'CHAPTER_MAP_UNAVAILABLE'
	| 'FAILED'
	| 'NO_AUDIOBOOK_PAIRED'
	| 'READY'
	| 'SYNC_MAP_UNAVAILABLE'
	| 'UNAVAILABLE'

export type BookRequestGatewayInput = {
	automationEnabled?: boolean
	enabled?: boolean
	endpoint: string
	handoffRoot?: string | null | undefined
	maxRetries?: number
	requireApproval?: boolean
	scoringFloor?: number
	token: string
	verificationThreshold?: number
}

export type BookRequestStatus =
	| 'APPROVED'
	| 'AWAITING_APPROVAL'
	| 'COMPLETED'
	| 'FAILED'
	| 'GRABBED'
	| 'IMPORTING'
	| 'NEEDS_SELECTION'
	| 'PENDING'
	| 'QUEUED'
	| 'REJECTED'
	| 'SEARCHING'

export type BookReviewInput = {
	content?: string | null | undefined
	isPrivate: boolean
	/** Zero means unrated; values above five are rejected. */
	rating: number
}

/**
 * Represents a collected issue/series within a TPB or GN
 * See https://github.com/mylar3/mylar3/wiki/series.json-schema-%28version-1.0.1%29
 */
export type CollectedItemInput = {
	/** CV ComicID of series */
	comicid?: string | null | undefined
	/** CV IssueID of single issue (not valid if multiple issues) */
	issueid?: string | null | undefined
	/** Listing of issue numbers present pertaining to related comicid in collection */
	issues?: string | null | undefined
	/** The title of the series */
	series?: string | null | undefined
}

export type ComputedFilterLibraryType =
	| { is: LibraryType; isAnyOf?: never; isNoneOf?: never; isNot?: never }
	| { is?: never; isAnyOf: Array<LibraryType>; isNoneOf?: never; isNot?: never }
	| { is?: never; isAnyOf?: never; isNoneOf: Array<LibraryType>; isNot?: never }
	| { is?: never; isAnyOf?: never; isNoneOf?: never; isNot: LibraryType }

export type ComputedFilterReadingStatus =
	| { is: ReadingStatus; isAnyOf?: never; isNoneOf?: never; isNot?: never }
	| { is?: never; isAnyOf: Array<ReadingStatus>; isNoneOf?: never; isNot?: never }
	| { is?: never; isAnyOf?: never; isNoneOf: Array<ReadingStatus>; isNot?: never }
	| { is?: never; isAnyOf?: never; isNoneOf?: never; isNot: ReadingStatus }

export type CreateBookClubInput = {
	creatorDisplayName?: string | null | undefined
	creatorHideProgress: boolean
	description?: string | null | undefined
	isPrivate?: boolean
	memberRoleSpec?: unknown
	name: string
	slug?: string | null | undefined
}

export type CreateBookRequestInput = {
	authors?: string | null | undefined
	automationEnabled?: boolean
	coverUrl?: string | null | undefined
	destinationDeviceId?: string | number | null | undefined
	destinationShelfId?: string | number | null | undefined
	external?: ExternalWorkReferenceInput | null | undefined
	mediaId?: string | number | null | undefined
	title?: string | null | undefined
	workId?: string | number | null | undefined
}

export type CreateOrUpdateLibraryInput = {
	config?: LibraryConfigInput | null | undefined
	description?: string | null | undefined
	emoji?: string | null | undefined
	name: string
	path: string
	scanAfterPersist?: boolean
	tags?: Array<string> | null | undefined
}

export type CreateShareGrantInput = {
	authors: string
	coverUrl?: string | null | undefined
	expiresAt?: string | null | undefined
	externalKey?: string | null | undefined
	recipientUserId: string | number
	recommendationId?: string | number | null | undefined
	remoteId?: string | null | undefined
	scopes: Array<ShareScope>
	sourceProvider?: string | null | undefined
	targetKey: string
	targetMediaId?: string | number | null | undefined
	targetWorkId?: string | number | null | undefined
	title: string
}

export type CrosspointDeliveryStatus =
	| 'CANCELLED'
	| 'COMPLETED'
	| 'FAILED'
	| 'PREPARING'
	| 'QUEUED'
	| 'TRANSFERRING'

/**
 * Verified CrossPoint endpoint settings and an optional typed profile patch.
 *
 * HTTP/WebSocket ports are accepted for the existing Home client contract but
 * are policy inputs, not routing inputs: mutations require exactly firmware
 * ports 80 and 81 and persist those constants rather than trusting a caller.
 */
export type CrosspointTargetInput = {
	hostOrIp: string
	httpPort: number
	profile?: CrosspointTransferProfileInput | null | undefined
	rootPath: string
	wsPort: number
}

export type CrosspointTargetModel = 'AUTO' | 'X3' | 'X4'

/**
 * A bounded, nullable patch for a CrossPoint transfer profile.
 *
 * The protocol crate owns defaults and validation. This type only bridges
 * GraphQL's `Int` scalar to the protocol's unsigned fields; callers must run
 * [`Self::into_protocol`] before persisting or queueing the profile.
 */
export type CrosspointTransferProfileInput = {
	autoCrop?: boolean | null | undefined
	chunkBytes?: number | null | undefined
	grayscale?: boolean | null | undefined
	jpegQuality?: number | null | undefined
	maxUploadBytes?: number | null | undefined
	optimizerEnabled?: boolean | null | undefined
	removeFonts?: boolean | null | undefined
	retryCount?: number | null | undefined
	retryDelaySeconds?: number | null | undefined
	splitLargeParagraphs?: boolean | null | undefined
	targetModel?: CrosspointTargetModel | null | undefined
	timeoutSeconds?: number | null | undefined
}

/** A simple cursor-based pagination input object */
export type CursorPagination = {
	after?: string | null | undefined
	limit?: number
}

/** The storage a device credential references */
export type DeviceCredentialKind =
	/** `credential_ref` is an `api_keys.short_token` */
	| 'API_KEY'
	/** `credential_ref` is a `liseur_sync_tokens.id` */
	| 'LISEUR_TOKEN'
	/** `credential_ref` is a `sessions.session_id` */
	| 'SESSION'

/**
 * The client family a registered device belongs to. The kind decides which
 * credential is minted for the device and which endpoints it is handed.
 */
export type DeviceKind =
	/** An Audiobookshelf client using the ABS-compatible profile */
	| 'ABS'
	/** A script or integration using the native API */
	| 'API'
	/** A Coppice Home/KOReader client managed through the unified device surface */
	| 'COPPICE'
	/** A CrossPoint Reader using stock KOSync plus the keyed rich-sync lane */
	| 'CROSSPOINT'
	/** A Kavita-compatible reader using the native API with a download-only key */
	| 'KAVITA'
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
	| 'WEB'
	/**
	 * A remote worker process (`stump-worker`) that claims `worker_jobs` over
	 * the worker socket. Not a reading client: it holds no reading state and
	 * is never offered a transformed stream.
	 */
	| 'WORKER'

/**
 * The lifecycle state of a device-pairing request. `Expired` is derived from
 * `expires_at` for pending rows and persisted lazily once observed, so a row's
 * stored status may still read `Pending` after the deadline; always go through
 * [`Model::effective_status`].
 */
export type DevicePairingStatus = 'APPROVED' | 'DENIED' | 'EXPIRED' | 'PENDING'

/** The wire protocol through which a device credential was exercised */
export type DeviceProtocol = 'API' | 'KOBO' | 'KOMGA' | 'KOREADER' | 'LISEUR' | 'OPDS'

export type Dimension = 'HEIGHT' | 'WIDTH'

/** The configuration for an [EmailerClient] */
export type EmailerClientConfig = {
	/** The SMTP host to use */
	host: string
	/** The maximum size of an attachment in bytes */
	maxAttachmentSizeBytes?: number | null | undefined
	/** The maximum number of attachments that can be sent in a single email */
	maxNumAttachments?: number | null | undefined
	/**
	 * The plaintext password to use for the SMTP server, which will be encrypted before being stored.
	 * This field is optional to support reusing the config for emailer config updates. If the password is not
	 * set, it will error when trying to send an email.
	 */
	password?: string | null | undefined
	/** The SMTP port to use */
	port: number
	/** The display name to use for the sender */
	senderDisplayName: string
	/** The email address to send from */
	senderEmail: string
	/** Whether to use TLS for the SMTP connection */
	tlsEnabled: boolean
	/** The username to use for the SMTP server, typically the same as the sender email */
	username: string
}

/** Input object for creating or updating an emailer */
export type EmailerInput = {
	/** The emailer configuration */
	config: EmailerClientConfig
	/** Whether the emailer is the primary emailer */
	isPrimary: boolean
	/** The friendly name of the emailer, e.g. "Aaron's Kobo" */
	name: string
}

export type EpubProgressInput = {
	deviceId?: string | null | undefined
	elapsedSecondsDelta?: number | null | undefined
	isComplete?: boolean | null | undefined
	locator: ReadiumLocatorInput
	percentage?: unknown
	resetElapsedSeconds?: boolean | null | undefined
}

/**
 * A resize option which will resize the image to the given dimensions, without
 * maintaining the aspect ratio.
 */
export type ExactDimensionResizeInput = {
	/** The height (in pixels) the resulting image should be resized to */
	height: number
	/** The width (in pixels) the resulting image should be resized to */
	width: number
}

/**
 * A catalog/work identity outside the Coppice library. The gateway only sees
 * normalized metadata and an opaque candidate id later returned by search.
 */
export type ExternalWorkReferenceInput = {
	authors?: string | null | undefined
	coverUrl?: string | null | undefined
	externalKey?: string | null | undefined
	remoteId: string
	sourceProvider: string
	title: string
}

export type FieldFilterFileStatus =
	| {
			anyOf: Array<FileStatus>
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains: FileStatus
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith: FileStatus
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq: FileStatus
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes: FileStatus
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like: FileStatus
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf: Array<FileStatus>
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf: Array<FileStatus>
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq: FileStatus
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf: Array<FileStatus>
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith: FileStatus
	  }

export type FieldFilterString =
	| {
			anyOf: Array<string>
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains: string
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith: string
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq: string
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes: string
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like: string
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf: Array<string>
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf: Array<string>
			neq?: never
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq: string
			noneOf?: never
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf: Array<string>
			startsWith?: never
	  }
	| {
			anyOf?: never
			contains?: never
			endsWith?: never
			eq?: never
			excludes?: never
			like?: never
			likeAnyOf?: never
			likeNoneOf?: never
			neq?: never
			noneOf?: never
			startsWith: string
	  }

/** The different statuses a file reference can have */
export type FileStatus = 'ERROR' | 'MISSING' | 'READY' | 'UNKNOWN' | 'UNSUPPORTED'

/**
 * A resize option which will resize the image to fit within the given dimensions,
 * maintaining the aspect ratio.
 *
 * If the image already fits within the dimensions, it will not be scaled up.
 */
export type FitWithinResizeInput = {
	/** The maximum height (in pixels) of the resulting image */
	height: number
	/** The maximum width (in pixels) of the resulting image */
	width: number
}

/** Options for processing images throughout Stump. */
export type ImageProcessorOptionsInput = {
	/** The format to use when generating an image. See [`SupportedImageFormat`] */
	format: SupportedImageFormat
	/** The page to use when generating an image. This is not applicable to all media formats. */
	page?: number | null | undefined
	/**
	 * The quality to use when generating an image. This is a number between 1 and 100,
	 * where 100 is the highest quality. Omitting this value will use the default quality
	 * of 100.
	 */
	quality?: number | null | undefined
	/** The size factor to use when generating an image. See [`ImageResizeOptions`] */
	resizeMethod?: ImageResizeMethodInput | null | undefined
}

/** The resize options to use when generating an image */
export type ImageResizeMethodInput =
	| {
			exact: ExactDimensionResizeInput
			fitWithin?: never
			scaleDimension?: never
			scaleEvenlyByFactor?: never
	  }
	| {
			exact?: never
			fitWithin: FitWithinResizeInput
			scaleDimension?: never
			scaleEvenlyByFactor?: never
	  }
	| {
			exact?: never
			fitWithin?: never
			scaleDimension: ScaledDimensionResizeInput
			scaleEvenlyByFactor?: never
	  }
	| {
			exact?: never
			fitWithin?: never
			scaleDimension?: never
			scaleEvenlyByFactor: ScaleEvenlyByFactorInput
	  }

export type IngestSettingValueType = 'BOOLEAN' | 'INTEGER' | 'JSON' | 'NUMBER' | 'STRING'

export type JobStatus = 'CANCELLED' | 'COMPLETED' | 'FAILED' | 'PAUSED' | 'QUEUED' | 'RUNNING'

export type LibraryConfigInput = {
	convertRarToZip: boolean
	defaultLibraryViewMode: LibraryViewMode
	defaultReadingDir: ReadingDirection
	defaultReadingImageScaleFit: ReadingImageScaleFit
	defaultReadingMode: ReadingMode
	generateFileHashes: boolean
	generateKoreaderHashes: boolean
	hardDeleteConversions: boolean
	hideSeriesView: boolean
	ignoreRules?: Array<string> | null | undefined
	libraryPattern: LibraryPattern
	libraryType: LibraryType
	oneshotsDirectory?: string | null | undefined
	processMetadata: boolean
	processThumbnailColorsEvenWithoutConfig: boolean
	skipBookOverview: boolean
	thumbnailConfig?: ImageProcessorOptionsInput | null | undefined
	watch: boolean
}

export type LibraryFilterInput = {
	_and?: Array<LibraryFilterInput> | null | undefined
	_not?: Array<LibraryFilterInput> | null | undefined
	_or?: Array<LibraryFilterInput> | null | undefined
	id?: FieldFilterString | null | undefined
	name?: FieldFilterString | null | undefined
	path?: FieldFilterString | null | undefined
}

/** The different patterns a library may be organized by */
export type LibraryPattern = 'COLLECTION_BASED' | 'SERIES_BASED'

/** The type of content a library contains */
export type LibraryType =
	| 'BOOK'
	| 'COMIC'
	| 'LIGHT_NOVEL'
	| 'MANGA'
	| 'MANHWA'
	| 'MIXED'
	| 'WEBTOON'
	| 'WEB_NOVEL'

export type LibraryViewMode = 'BOOKS' | 'SERIES'

export type MediaFilterInput = {
	_and?: Array<MediaFilterInput> | null | undefined
	_not?: Array<MediaFilterInput> | null | undefined
	_or?: Array<MediaFilterInput> | null | undefined
	createdAt?: NumericFilterDateTime | null | undefined
	extension?: FieldFilterString | null | undefined
	id?: FieldFilterString | null | undefined
	metadata?: MediaMetadataFilterInput | null | undefined
	name?: FieldFilterString | null | undefined
	pages?: NumericFilterI32 | null | undefined
	path?: FieldFilterString | null | undefined
	readingStatus?: ComputedFilterReadingStatus | null | undefined
	series?: SeriesFilterInput | null | undefined
	seriesId?: FieldFilterString | null | undefined
	size?: NumericFilterI64 | null | undefined
	status?: FieldFilterFileStatus | null | undefined
	tags?: FieldFilterString | null | undefined
	updatedAt?: NumericFilterDateTime | null | undefined
}

export type MediaMetadataFilterInput = {
	_and?: Array<MediaMetadataFilterInput> | null | undefined
	_not?: Array<MediaMetadataFilterInput> | null | undefined
	_or?: Array<MediaMetadataFilterInput> | null | undefined
	ageRating?: NumericFilterI32 | null | undefined
	characters?: FieldFilterString | null | undefined
	colorists?: FieldFilterString | null | undefined
	coverArtists?: FieldFilterString | null | undefined
	day?: NumericFilterI32 | null | undefined
	editors?: FieldFilterString | null | undefined
	genres?: FieldFilterString | null | undefined
	inkers?: FieldFilterString | null | undefined
	letterers?: FieldFilterString | null | undefined
	links?: FieldFilterString | null | undefined
	month?: NumericFilterI32 | null | undefined
	pencillers?: FieldFilterString | null | undefined
	publisher?: FieldFilterString | null | undefined
	series?: FieldFilterString | null | undefined
	summary?: FieldFilterString | null | undefined
	teams?: FieldFilterString | null | undefined
	title?: FieldFilterString | null | undefined
	writers?: FieldFilterString | null | undefined
	year?: NumericFilterI32 | null | undefined
}

export type MediaMetadataInput = {
	ageRating?: number | null | undefined
	characters?: Array<string> | null | undefined
	colorists?: Array<string> | null | undefined
	coverArtists?: Array<string> | null | undefined
	day?: number | null | undefined
	editors?: Array<string> | null | undefined
	format?: string | null | undefined
	genres?: Array<string> | null | undefined
	identifierAmazon?: string | null | undefined
	identifierCalibre?: string | null | undefined
	identifierGoogle?: string | null | undefined
	identifierIsbn?: string | null | undefined
	identifierMobiAsin?: string | null | undefined
	identifierUuid?: string | null | undefined
	inkers?: Array<string> | null | undefined
	language?: string | null | undefined
	letterers?: Array<string> | null | undefined
	links?: Array<string> | null | undefined
	month?: number | null | undefined
	/**
	 * The audiobook's readers. A separate credit from `writers`: an
	 * audiobook's author wrote it and its narrator did not.
	 */
	narrators?: Array<string> | null | undefined
	notes?: string | null | undefined
	number?: unknown
	pageCount?: number | null | undefined
	pencillers?: Array<string> | null | undefined
	publisher?: string | null | undefined
	series?: string | null | undefined
	seriesGroup?: string | null | undefined
	storyArc?: string | null | undefined
	storyArcNumber?: unknown
	summary?: string | null | undefined
	teams?: Array<string> | null | undefined
	title?: string | null | undefined
	titleSort?: string | null | undefined
	volume?: number | null | undefined
	writers?: Array<string> | null | undefined
	year?: number | null | undefined
}

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
	| 'NARRATORS'
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
	| 'YEAR'

export type MediaMetadataOrderByField = {
	direction: OrderDirection
	field: MediaMetadataModelOrdering
}

/**
 * A manual override for searching metadata providers for a single media item. When
 * provided, the caller's fields take precedence over whatever is already stored on the
 * media, and the search is restricted to a single provider if one is given.
 */
export type MediaMetadataSearchInput = {
	author?: string | null | undefined
	/**
	 * The volume ID to search within, which will swap to a more precise lookup if provided alongside
	 * `number`
	 */
	comicVineVolumeId?: string | null | undefined
	isbn?: string | null | undefined
	limit?: number | null | undefined
	/** The issue number (for comics/manga) */
	number?: number | null | undefined
	/**
	 * Restrict the search to this provider only. If omitted, all enabled providers
	 * configured for the media's library type are searched.
	 */
	provider?: MetadataProvider | null | undefined
	title?: string | null | undefined
	year?: number | null | undefined
}

export type MediaModelOrdering =
	| 'CREATED_AT'
	| 'DELETED_AT'
	| 'EXTENSION'
	| 'HASH'
	| 'ID'
	| 'IS_ONESHOT'
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
	| 'UPDATED_AT'

export type MediaOrderBy =
	| { media: MediaOrderByField; metadata?: never }
	| { media?: never; metadata: MediaMetadataOrderByField }

export type MediaOrderByField = {
	direction: OrderDirection
	field: MediaModelOrdering
}

export type MediaProgressInput =
	| { audio: AudioProgressInput; epub?: never; paged?: never }
	| { audio?: never; epub: EpubProgressInput; paged?: never }
	| { audio?: never; epub?: never; paged: PagedProgressInput }

export type MetadataFetchStatus =
	| 'AWAITING_REVIEW'
	| 'FAILED'
	| 'FETCHED'
	| 'IN_PROGRESS'
	| 'MATCHED'
	| 'NOT_STARTED'
	| 'NO_MATCH'
	| 'RATE_LIMITED'

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
	/**
	 * The readers of an audiobook edition, carried by
	 * [`crate::types::ExternalMediaMetadata::narrators`]. Distinct from
	 * [`Self::Writers`] on purpose -- a narrator is not an author.
	 */
	| 'NARRATORS'
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
	/**
	 * An edition's secondary title, carried by
	 * [`crate::types::ExternalMediaMetadata::subtitle`].
	 */
	| 'SUBTITLE'
	| 'SUMMARY'
	| 'TAGS'
	| 'TEAMS'
	| 'TITLE'
	| 'TITLE_SORT'
	| 'VOLUME_COUNT'
	| 'WRITERS'
	| 'YEAR'

/** The supported external metadata providers */
export type MetadataProvider =
	/** AniList (https://anilist.co) */
	| 'ANI_LIST'
	/**
	 * Audible, through the community Audnexus enrichment API
	 * (https://api.audnex.us) plus the unauthenticated Audible catalogue.
	 */
	| 'AUDIBLE'
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
	| 'OPEN_LIBRARY'

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
	/** A recipient accepted a recommendation or share grant. */
	| 'RECOMMENDATION_ACCEPTED'
	/** A recipient declined a recommendation or share grant. */
	| 'RECOMMENDATION_DECLINED'
	/** A social recommendation or explicit share was sent to this user. */
	| 'RECOMMENDATION_RECEIVED'
	/** A sender or authorized owner revoked a recommendation or share grant. */
	| 'RECOMMENDATION_REVOKED'
	/** A recipient's request handoff changed state. */
	| 'REQUEST_STATUS_UPDATED'
	/** A library scan completed */
	| 'SCAN_FINISHED'
	/** Sent by `testNotificationChannel`; never routed by rules */
	| 'TEST'

export type NumericFilterDateTime =
	| {
			anyOf: Array<string>
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq: string
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt: string
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte: string
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt: string
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte: string
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq: string
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf: Array<string>
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range: NumericRangeDateTime
	  }

export type NumericFilterI32 =
	| {
			anyOf: Array<number>
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq: number
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt: number
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte: number
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt: number
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte: number
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq: number
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf: Array<number>
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range: NumericRangeI32
	  }

export type NumericFilterI64 =
	| {
			anyOf: Array<number>
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq: number
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt: number
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte: number
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt: number
			lte?: never
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte: number
			neq?: never
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq: number
			noneOf?: never
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf: Array<number>
			range?: never
	  }
	| {
			anyOf?: never
			eq?: never
			gt?: never
			gte?: never
			lt?: never
			lte?: never
			neq?: never
			noneOf?: never
			range: NumericRangeI64
	  }

export type NumericRangeDateTime = {
	from: string
	inclusive: boolean
	to: string
}

export type NumericRangeI32 = {
	from: number
	inclusive: boolean
	to: number
}

export type NumericRangeI64 = {
	from: number
	inclusive: boolean
	to: number
}

/** A simple offset-based pagination input object */
export type OffsetPagination = {
	/**
	 * The page to start from. This is 1-based by default, but can be
	 * changed to 0-based by setting the `zero_based` field to true.
	 */
	page: number
	/** The number of items to return per page. This is 20 by default. */
	pageSize?: number | null | undefined
	/** Whether or not the page is zero-based. This is false by default. */
	zeroBased?: boolean | null | undefined
}

export type OrderDirection = 'ASC' | 'DESC'

export type PagedProgressInput = {
	deviceId?: string | null | undefined
	elapsedSecondsDelta?: number | null | undefined
	page: number
	resetElapsedSeconds?: boolean | null | undefined
}

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
	| { cursor: CursorPagination; none?: never; offset?: never }
	| { cursor?: never; none: Unpaginated; offset?: never }
	| { cursor?: never; none?: never; offset: OffsetPagination }

/**
 * Why pairing believes two media rows are the same work.
 *
 * Ordered by strength through [`PairEvidence::rank`], which is what keeps a
 * title guess from overwriting an identifier match on the same link.
 */
export type PairEvidence =
	/** An operator paired them by hand. */
	| 'MANUAL'
	/**
	 * An identifier of one matched an identifier of the other through a
	 * provider's edition list (Audnexus `/books/{asin}`, Open Library
	 * works→editions).
	 */
	| 'PROVIDER_EDITION_LIST'
	/** Both files arrived in one ingest drop group. */
	| 'SAME_DROP'
	/**
	 * Normalised `title + first author` matched. The weakest signal, and the
	 * reason confirmation exists at all.
	 */
	| 'TITLE_AUTHOR'
	/** Both media rows already linked to the same work. */
	| 'WORK_ID'

/**
 * Whether a link is an asserted edition of the work, a guess, or a guess the
 * user refused.
 */
export type PairStatus =
	/**
	 * An edition of the work. The schema default, so every link the liseur
	 * lane wrote — a client asserting a work for a file whose edition digest
	 * it computed — keeps meaning what it always did. This is what
	 * `Media.editions` returns.
	 */
	| 'CONFIRMED'
	/**
	 * The user said no. Kept as a row rather than deleted, because the
	 * suggestion is recomputed on every book-page query.
	 */
	| 'REJECTED'
	/** A heuristic match awaiting confirmation. Never presented as an edition. */
	| 'SUGGESTED'

/** The different reading directions supported by any Stump reader */
export type ReadingDirection = 'LTR' | 'RTL'

/** The different ways an image may be scaled to fit a reader's viewport */
export type ReadingImageScaleFit = 'AUTO' | 'HEIGHT' | 'NONE' | 'WIDTH'

/** The different reading modes supported by any Stump reader */
export type ReadingMode = 'CONTINUOUS_HORIZONTAL' | 'CONTINUOUS_VERTICAL' | 'PAGED'

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
	| 'YEAR'

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
	| 'READING'

export type ReadiumLocationInput = {
	cssSelector?: string | null | undefined
	fragments?: Array<string> | null | undefined
	partialCfi?: string | null | undefined
	position?: number | null | undefined
	progression?: unknown
	totalProgression?: unknown
}

export type ReadiumLocatorInput = {
	chapterTitle?: string
	href: string
	koboSpan?: string | null | undefined
	locations?: ReadiumLocationInput | null | undefined
	text?: ReadiumTextInput | null | undefined
	title?: string | null | undefined
	type?: string
}

export type ReadiumTextInput = {
	after?: string | null | undefined
	before?: string | null | undefined
	highlight?: string | null | undefined
}

export type RecommendationDirection = 'INCOMING' | 'OUTGOING'

export type RecommendationHandoffState = 'FAILED' | 'LINKED' | 'NONE' | 'REQUESTED'

export type RecommendationResponseInput = {
	accept: boolean
}

export type RecommendationState =
	| 'ACCEPTED'
	| 'DECLINED'
	| 'DISMISSED'
	| 'EXPIRED'
	| 'PENDING'
	| 'REVOKED'

export type RecommendationTargetKind = 'EXTERNAL_WORK' | 'INTERNAL_MEDIA' | 'INTERNAL_WORK'

export type RequestDestinationInput = {
	deviceId?: string | number | null | undefined
	shelfId?: string | number | null | undefined
}

/**
 * The kind of an owned resource gauge. Total process RSS is not a component
 * gauge and is exposed separately through [`RuntimeMemory`].
 */
export type RuntimeGaugeKind = 'BYTES' | 'COUNT'

/** Whether a component toggle takes effect immediately or at process restart. */
export type RuntimeTransitionMode = 'HOT' | 'RESTART'

/** Evidence-backed activity state for a compiled component. */
export type RuntimeUsageStatus = 'UNKNOWN' | 'UNUSED' | 'USED'

export type ScaleEvenlyByFactorInput = {
	/**
	 * The factor to scale the image by. Note that this was made a [Decimal]
	 * to correct precision issues
	 */
	factor: unknown
}

/**
 * A resize option which will resize the image while maintaining the aspect ratio.
 * The dimension *not* specified will be calculated based on the aspect ratio.
 */
export type ScaledDimensionResizeInput = {
	/** The dimension to set with the given size, e.g. `Height` or `Width`. */
	dimension: Dimension
	/** The size (in pixels) to set the specified dimension to. */
	size: number
}

export type SendRecommendationInput = {
	authors: string
	coverUrl?: string | null | undefined
	externalKey?: string | null | undefined
	mediaId?: string | number | null | undefined
	message?: string | null | undefined
	recipientUserId: string | number
	remoteId?: string | null | undefined
	sourceProvider?: string | null | undefined
	targetKind: RecommendationTargetKind
	title: string
	workId?: string | number | null | undefined
}

export type SeriesFilterInput = {
	_and?: Array<SeriesFilterInput> | null | undefined
	_not?: Array<SeriesFilterInput> | null | undefined
	_or?: Array<SeriesFilterInput> | null | undefined
	isOneshot?: boolean | null | undefined
	library?: LibraryFilterInput | null | undefined
	libraryId?: FieldFilterString | null | undefined
	libraryType?: ComputedFilterLibraryType | null | undefined
	metadata?: SeriesMetadataFilterInput | null | undefined
	name?: FieldFilterString | null | undefined
	path?: FieldFilterString | null | undefined
	readingStatus?: ComputedFilterReadingStatus | null | undefined
}

export type SeriesMetadataFilterInput = {
	_and?: Array<SeriesMetadataFilterInput> | null | undefined
	_not?: Array<SeriesMetadataFilterInput> | null | undefined
	_or?: Array<SeriesMetadataFilterInput> | null | undefined
	ageRating?: NumericFilterI32 | null | undefined
	booktype?: FieldFilterString | null | undefined
	comicid?: NumericFilterI32 | null | undefined
	imprint?: FieldFilterString | null | undefined
	metaType?: FieldFilterString | null | undefined
	publisher?: FieldFilterString | null | undefined
	status?: FieldFilterString | null | undefined
	summary?: FieldFilterString | null | undefined
	title?: FieldFilterString | null | undefined
	volume?: NumericFilterI32 | null | undefined
	year?: NumericFilterI32 | null | undefined
}

export type SeriesMetadataInput = {
	ageRating?: number | null | undefined
	booktype?: string | null | undefined
	characters?: Array<string> | null | undefined
	collects?: Array<CollectedItemInput> | null | undefined
	comicImage?: string | null | undefined
	comicid?: number | null | undefined
	descriptionFormatted?: string | null | undefined
	genres?: Array<string> | null | undefined
	imprint?: string | null | undefined
	links?: Array<string> | null | undefined
	metaType?: string | null | undefined
	publicationRun?: string | null | undefined
	publisher?: string | null | undefined
	status?: string | null | undefined
	summary?: string | null | undefined
	title?: string | null | undefined
	totalIssues?: number | null | undefined
	volume?: number | null | undefined
	writers?: Array<string> | null | undefined
	year?: number | null | undefined
}

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
	| 'YEAR'

export type SeriesMetadataOrderByField = {
	direction: OrderDirection
	field: SeriesMetadataModelOrdering
}

export type SeriesModelOrdering =
	| 'CREATED_AT'
	| 'DELETED_AT'
	| 'DESCRIPTION'
	| 'ID'
	| 'IS_ONESHOT'
	| 'LIBRARY_ID'
	| 'NAME'
	| 'PATH'
	| 'REMOTE_ID'
	| 'SOURCE_PROVIDER'
	| 'STATUS'
	| 'THUMBNAIL_META'
	| 'THUMBNAIL_PATH'
	| 'UPDATED_AT'

export type SeriesOrderBy =
	| { metadata: SeriesMetadataOrderByField; series?: never }
	| { metadata?: never; series: SeriesOrderByField }

export type SeriesOrderByField = {
	direction: OrderDirection
	field: SeriesModelOrdering
}

export type SetNotificationChannelSettingsInput = {
	/** The channel whose per-user settings are written. */
	channelId: string
	/**
	 * Values keyed by the channel's settings definition keys. Stored values
	 * are merged over the channel defaults; omitted secret keys keep their
	 * previously stored (encrypted) value.
	 */
	settings: unknown
}

export type ShareOverlayKind = 'BOOKMARK' | 'HIGHLIGHT' | 'NOTE'

export type ShareScope =
	| 'ANNOTATIONS'
	| 'COMPLETION'
	| 'METADATA'
	| 'PERCENTAGE'
	| 'RATING'
	| 'REVIEW_TEXT'

export type ShareState = 'ACTIVE' | 'DECLINED' | 'EXPIRED' | 'PENDING' | 'REVOKED'

/** Supported image formats for processing images throughout Stump */
export type SupportedImageFormat = 'JPEG' | 'PNG' | 'WEBP'

/**
 * A simple pagination input object which does not paginate. An explicit struct is
 * required as a limitation of async_graphql's [OneofObject], which doesn't allow
 * for empty variants.
 */
export type Unpaginated = {
	unpaginated: boolean
}

export type UpdateAnnotationInput = {
	annotationText?: string | null | undefined
	id: string
}

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
	/**
	 * Grant a device the right to act as a remote worker: open the worker
	 * socket, claim `worker_jobs`, and upload their outputs
	 */
	| 'ACCESS_WORKER'
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
	| 'WRITE_BACK_METADATA'

/**
 * Where a [`crate::entity::worker_job`] row is in its life.
 *
 * `needs_worker` is a resting state, not an error: the job is well-formed and
 * nobody who can run it is connected. It becomes `queued` again the moment a
 * capable worker says `hello`. There is deliberately no `cancelled`: the
 * protocol's status set is its contract, and a cancelled job is exactly a job
 * that will not produce its output, so it is `failed` with a reason.
 */
export type WorkerJobStatus =
	/** A worker answered the offer with `claim`. */
	| 'CLAIMED'
	/** The job produced its output. */
	| 'DONE'
	/** The job did not produce its output; `error` says why. */
	| 'FAILED'
	/**
	 * No connected worker advertises what this job needs and the kind has no
	 * local implementation. Shown, never hidden.
	 */
	| 'NEEDS_WORKER'
	/** Created, and either not yet offered or offered and not yet claimed. */
	| 'QUEUED'
	/** The job is executing — remotely (first `progress` frame) or locally. */
	| 'RUNNING'

export type ConsoleAnnotationFieldsFragment = {
	id: string
	kind: AnnotationKind
	source: DeviceKind
	sourceDeviceId: string | null
	sourceDeviceName: string | null
	editable: boolean
	chapterTitle: string | null
	href: string | null
	fragment: string | null
	page: number | null
	progression: number | null
	excerpt: string | null
	note: string | null
	color: string | null
	createdAt: string | null
	updatedAt: string | null
	book: {
		key: string
		mediaId: string | null
		title: string
		authors: Array<string>
		seriesId: string | null
		seriesName: string | null
		libraryId: string | null
		extension: string | null
	}
}

export type ConsoleAnnotationsQueryVariables = Exact<{
	filter?: AnnotationFilterInput | null | undefined
	pagination?: OffsetPagination | null | undefined
}>

export type ConsoleAnnotationsQuery = {
	annotations: {
		total: number
		bookCount: number
		hasNext: boolean
		items: Array<{
			id: string
			kind: AnnotationKind
			source: DeviceKind
			sourceDeviceId: string | null
			sourceDeviceName: string | null
			editable: boolean
			chapterTitle: string | null
			href: string | null
			fragment: string | null
			page: number | null
			progression: number | null
			excerpt: string | null
			note: string | null
			color: string | null
			createdAt: string | null
			updatedAt: string | null
			book: {
				key: string
				mediaId: string | null
				title: string
				authors: Array<string>
				seriesId: string | null
				seriesName: string | null
				libraryId: string | null
				extension: string | null
			}
		}>
	}
}

export type ConsoleAnnotationBooksQueryVariables = Exact<{
	filter?: AnnotationFilterInput | null | undefined
}>

export type ConsoleAnnotationBooksQuery = {
	annotations: { items: Array<{ book: { key: string; mediaId: string | null; title: string } }> }
}

export type ConsoleUpdateAnnotationMutationVariables = Exact<{
	input: UpdateAnnotationInput
}>

export type ConsoleUpdateAnnotationMutation = {
	updateAnnotation: { id: string; annotationText: string | null; updatedAt: string }
}

export type ConsoleDeleteAnnotationMutationVariables = Exact<{
	id: string
}>

export type ConsoleDeleteAnnotationMutation = { deleteAnnotation: { id: string } }

export type ConsoleAnnotationSinksQueryVariables = Exact<{ [key: string]: never }>

export type ConsoleAnnotationSinksQuery = {
	annotationSinks: Array<{
		id: string
		name: string
		description: string
		settings: Array<{
			key: string
			label: string
			valueType: IngestSettingValueType
			required: boolean
			secret: boolean
			defaultValue: unknown
			description: string | null
			helpUrl: string | null
		}>
	}>
	annotationSyncStatus: {
		userId: string
		pending: boolean
		sinks: Array<{
			sinkId: string
			enabled: boolean
			lastRunAt: string | null
			lastError: string | null
		}>
	}
}

export type ConsoleSetAnnotationSinkSettingsMutationVariables = Exact<{
	sinkId: string
	settings?: unknown
	enabled?: boolean | null | undefined
}>

export type ConsoleSetAnnotationSinkSettingsMutation = {
	setAnnotationSinkSettings: {
		userId: string
		pending: boolean
		sinks: Array<{
			sinkId: string
			enabled: boolean
			lastRunAt: string | null
			lastError: string | null
		}>
	}
}

export type ConsoleRunAnnotationSyncMutationVariables = Exact<{ [key: string]: never }>

export type ConsoleRunAnnotationSyncMutation = {
	runAnnotationSync: {
		userId: string
		pending: boolean
		sinks: Array<{
			sinkId: string
			enabled: boolean
			lastRunAt: string | null
			lastError: string | null
		}>
	}
}

export type BookDetailMetadataFieldsFragment = {
	id: number
	title: string | null
	titleSort: string | null
	series: string | null
	seriesGroup: string | null
	storyArc: string | null
	storyArcNumber: unknown
	number: unknown
	volume: number | null
	summary: string | null
	notes: string | null
	genres: Array<string>
	format: string | null
	year: number | null
	month: number | null
	day: number | null
	writers: Array<string>
	pencillers: Array<string>
	inkers: Array<string>
	colorists: Array<string>
	letterers: Array<string>
	coverArtists: Array<string>
	editors: Array<string>
	narrators: Array<string>
	publisher: string | null
	links: Array<string>
	characters: Array<string>
	teams: Array<string>
	pageCount: number | null
	ageRating: number | null
	identifierAmazon: string | null
	identifierCalibre: string | null
	identifierGoogle: string | null
	identifierIsbn: string | null
	identifierMobiAsin: string | null
	identifierUuid: string | null
	language: string | null
	metadataSource: string | null
	metadataExternalId: string | null
	lockedFields: Array<MetadataField>
}

export type BookDetailAudioFieldsFragment = {
	durationMs: number
	codec: string
	sampleRate: number | null
	channels: number | null
	bitrate: number | null
	chapterSource: string
	chapters: Array<{
		id: string
		index: number
		title: string | null
		startMs: number
		endMs: number | null
	}>
}

export type BookDetailFileFieldsFragment = {
	path: string
	size: number
	extension: string
	hash: string | null
	koreaderHash: string | null
	status: FileStatus
	modifiedAt: string | null
}

export type BookDetailEditionFieldsFragment = {
	mediaId: string
	kind: BookEditionKind
	title: string
	pairEvidence: PairEvidence | null
	metadata: {
		id: number
		title: string | null
		titleSort: string | null
		series: string | null
		seriesGroup: string | null
		storyArc: string | null
		storyArcNumber: unknown
		number: unknown
		volume: number | null
		summary: string | null
		notes: string | null
		genres: Array<string>
		format: string | null
		year: number | null
		month: number | null
		day: number | null
		writers: Array<string>
		pencillers: Array<string>
		inkers: Array<string>
		colorists: Array<string>
		letterers: Array<string>
		coverArtists: Array<string>
		editors: Array<string>
		narrators: Array<string>
		publisher: string | null
		links: Array<string>
		characters: Array<string>
		teams: Array<string>
		pageCount: number | null
		ageRating: number | null
		identifierAmazon: string | null
		identifierCalibre: string | null
		identifierGoogle: string | null
		identifierIsbn: string | null
		identifierMobiAsin: string | null
		identifierUuid: string | null
		language: string | null
		metadataSource: string | null
		metadataExternalId: string | null
		lockedFields: Array<MetadataField>
	} | null
	file: {
		path: string
		size: number
		extension: string
		hash: string | null
		koreaderHash: string | null
		status: FileStatus
		modifiedAt: string | null
	}
	audio: {
		durationMs: number
		codec: string
		sampleRate: number | null
		channels: number | null
		bitrate: number | null
		chapterSource: string
		chapters: Array<{
			id: string
			index: number
			title: string | null
			startMs: number
			endMs: number | null
		}>
	} | null
}

export type BookDetailReviewFieldsFragment = {
	id: string
	rating: number
	content: string | null
	isPrivate: boolean
	mediaId: string | null
	workId: string | null
	createdAt: string
	updatedAt: string
}

export type BookDetailReadAloudFieldsFragment = {
	status: BookReadAloudStatus
	reason: string | null
	ebookMediaId: string | null
	audioMediaId: string | null
	chapterMap: Array<{
		ebookMediaId: string
		audioMediaId: string
		ebookSpineIndex: number
		audioChapterIndex: number
		confidence: number
	}>
	syncMap: {
		id: string
		ebookMediaId: string
		audioMediaId: string
		granularity: AlignGranularity
		generator: string
		generatorVersion: string
		algorithm: string | null
		model: string | null
		cueCount: number
		coverage: number | null
		source: string
		jobId: string | null
		createdAt: string
		map: unknown
	} | null
	artifact: { url: string; mimeType: string; cacheKey: string } | null
}

export type BookDetailQueryVariables = Exact<{
	mediaId: string | number
}>

export type BookDetailQuery = {
	bookDetail: {
		mediaId: string
		workId: string | null
		title: string
		authors: Array<string>
		workMetadata: {
			workId: string
			title: string | null
			author: string | null
			metadata: unknown
			lockedFields: Array<MetadataField>
		} | null
		editions: Array<{
			mediaId: string
			kind: BookEditionKind
			title: string
			pairEvidence: PairEvidence | null
			metadata: {
				id: number
				title: string | null
				titleSort: string | null
				series: string | null
				seriesGroup: string | null
				storyArc: string | null
				storyArcNumber: unknown
				number: unknown
				volume: number | null
				summary: string | null
				notes: string | null
				genres: Array<string>
				format: string | null
				year: number | null
				month: number | null
				day: number | null
				writers: Array<string>
				pencillers: Array<string>
				inkers: Array<string>
				colorists: Array<string>
				letterers: Array<string>
				coverArtists: Array<string>
				editors: Array<string>
				narrators: Array<string>
				publisher: string | null
				links: Array<string>
				characters: Array<string>
				teams: Array<string>
				pageCount: number | null
				ageRating: number | null
				identifierAmazon: string | null
				identifierCalibre: string | null
				identifierGoogle: string | null
				identifierIsbn: string | null
				identifierMobiAsin: string | null
				identifierUuid: string | null
				language: string | null
				metadataSource: string | null
				metadataExternalId: string | null
				lockedFields: Array<MetadataField>
			} | null
			file: {
				path: string
				size: number
				extension: string
				hash: string | null
				koreaderHash: string | null
				status: FileStatus
				modifiedAt: string | null
			}
			audio: {
				durationMs: number
				codec: string
				sampleRate: number | null
				channels: number | null
				bitrate: number | null
				chapterSource: string
				chapters: Array<{
					id: string
					index: number
					title: string | null
					startMs: number
					endMs: number | null
				}>
			} | null
		}>
		mismatches: Array<{
			field: MetadataField
			ebookValue: string | null
			audiobookValue: string | null
			workValue: string | null
			resolved: boolean
		}>
		review: {
			id: string
			rating: number
			content: string | null
			isPrivate: boolean
			mediaId: string | null
			workId: string | null
			createdAt: string
			updatedAt: string
		} | null
		readAloud: {
			status: BookReadAloudStatus
			reason: string | null
			ebookMediaId: string | null
			audioMediaId: string | null
			chapterMap: Array<{
				ebookMediaId: string
				audioMediaId: string
				ebookSpineIndex: number
				audioChapterIndex: number
				confidence: number
			}>
			syncMap: {
				id: string
				ebookMediaId: string
				audioMediaId: string
				granularity: AlignGranularity
				generator: string
				generatorVersion: string
				algorithm: string | null
				model: string | null
				cueCount: number
				coverage: number | null
				source: string
				jobId: string | null
				createdAt: string
				map: unknown
			} | null
			artifact: { url: string; mimeType: string; cacheKey: string } | null
		}
	} | null
}

export type BookCoverQueryVariables = Exact<{
	id: string | number
}>

export type BookCoverQuery = { mediaById: { thumbnail: { url: string } } | null }

export type BookReadingLogDeviceFieldsFragment = {
	id: string
	name: string | null
	kind: DeviceKind | null
	revoked: boolean
}

export type BookSearchQueryVariables = Exact<{
	query: string
	limit?: number | null | undefined
}>

export type BookSearchQuery = {
	searchBooks: Array<{
		mediaId: string
		workId: string | null
		title: string
		authors: Array<string>
		kind: BookEditionKind
		score: number
	}>
}

export type BookReadingLogQueryVariables = Exact<{
	mediaId: string | number
}>

export type BookReadingLogQuery = {
	bookReadingLog: {
		mediaId: string
		workId: string | null
		editions: Array<{
			mediaId: string
			kind: BookEditionKind
			head: {
				progression: number
				page: number | null
				positionMs: number | null
				trackIndex: number | null
				completed: boolean
				updatedAt: string
				sourceProtocol: string
				sourceDeviceId: string | null
				locator: {
					chapterTitle: string
					href: string
					title: string | null
					type: string
					locations: {
						fragments: Array<string> | null
						progression: unknown
						position: number | null
						totalProgression: unknown
						cssSelector: string | null
						partialCfi: string | null
					} | null
					text: { before: string | null; highlight: string | null; after: string | null } | null
				} | null
				sourceDevice: {
					id: string
					name: string | null
					kind: DeviceKind | null
					revoked: boolean
				} | null
			} | null
			sessions: Array<{
				id: string
				sessionDate: string
				status: string
				readthroughNumber: number
				startPage: number | null
				endPage: number | null
				endPositionMs: number | null
				startPercentage: number | null
				endPercentage: number | null
				elapsedSeconds: number | null
				notes: string | null
				sourceProtocol: string
				sourceDeviceIds: Array<string>
				liseurSessionId: string | null
				createdAt: string
				updatedAt: string | null
				startLocator: {
					chapterTitle: string
					href: string
					title: string | null
					type: string
					locations: {
						fragments: Array<string> | null
						progression: unknown
						position: number | null
						totalProgression: unknown
						cssSelector: string | null
						partialCfi: string | null
					} | null
					text: { before: string | null; highlight: string | null; after: string | null } | null
				} | null
				endLocator: {
					chapterTitle: string
					href: string
					title: string | null
					type: string
					locations: {
						fragments: Array<string> | null
						progression: unknown
						position: number | null
						totalProgression: unknown
						cssSelector: string | null
						partialCfi: string | null
					} | null
					text: { before: string | null; highlight: string | null; after: string | null } | null
				} | null
				sourceDevices: Array<{
					id: string
					name: string | null
					kind: DeviceKind | null
					revoked: boolean
				}>
			}>
		}>
	} | null
}

export type BookHighlightsQueryVariables = Exact<{
	mediaId: string | number
}>

export type BookHighlightsQuery = {
	annotations: {
		items: Array<{
			id: string
			kind: AnnotationKind
			source: DeviceKind
			sourceDeviceId: string | null
			sourceDeviceName: string | null
			editable: boolean
			chapterTitle: string | null
			href: string | null
			fragment: string | null
			page: number | null
			progression: number | null
			excerpt: string | null
			note: string | null
			color: string | null
			createdAt: string | null
			updatedAt: string | null
			book: { mediaId: string | null; title: string }
		}>
	}
}

export type SimilarBooksQueryVariables = Exact<{
	mediaId: string | number
	limit?: number | null | undefined
}>

export type SimilarBooksQuery = {
	similarBooks: Array<{
		mediaId: string
		workId: string | null
		title: string
		authors: Array<string>
		kind: BookEditionKind
		score: number
	}>
}

export type BookMetadataCandidatesQueryVariables = Exact<{
	mediaId: string | number
}>

export type BookMetadataCandidatesQuery = {
	bookMetadataCandidates: {
		id: number
		status: MetadataFetchStatus
		mediaId: string | null
		rawHits: number
		addedAt: string
		updatedAt: string | null
		matchCandidates: Array<{
			provider: string
			externalId: string
			confidence: number
			metadata:
				| {
						provider: string
						externalId: string
						title: string | null
						summary: string | null
						pageCount: number | null
						seriesName: string | null
						number: number | null
						day: number | null
						month: number | null
						year: number | null
						genres: Array<string> | null
						tags: Array<string> | null
						isbn: string | null
						isbn13: string | null
						writers: Array<string> | null
						artists: Array<string> | null
						colorists: Array<string> | null
						letterers: Array<string> | null
						coverArtists: Array<string> | null
						coverUrl: string | null
						providerUrl: string | null
						subtitle: string | null
						narrators: Array<string> | null
						publisher: string | null
						runtimeMinutes: number | null
				  }
				| Record<PropertyKey, never>
		}>
	} | null
}

export type SearchBookMetadataQueryVariables = Exact<{
	mediaId: string | number
	search?: MediaMetadataSearchInput | null | undefined
}>

export type SearchBookMetadataQuery = {
	searchBookMetadata: {
		id: number
		status: MetadataFetchStatus
		mediaId: string | null
		rawHits: number
		addedAt: string
		updatedAt: string | null
		matchCandidates: Array<{
			provider: string
			externalId: string
			confidence: number
			metadata:
				| {
						provider: string
						externalId: string
						title: string | null
						summary: string | null
						pageCount: number | null
						seriesName: string | null
						number: number | null
						day: number | null
						month: number | null
						year: number | null
						genres: Array<string> | null
						tags: Array<string> | null
						isbn: string | null
						isbn13: string | null
						writers: Array<string> | null
						artists: Array<string> | null
						colorists: Array<string> | null
						letterers: Array<string> | null
						coverArtists: Array<string> | null
						coverUrl: string | null
						providerUrl: string | null
						subtitle: string | null
						narrators: Array<string> | null
						publisher: string | null
						runtimeMinutes: number | null
				  }
				| Record<PropertyKey, never>
		}>
	}
}

export type ApplyBookMetadataMutationVariables = Exact<{
	mediaId: string | number
	input: BookMetadataApplyInput
}>

export type ApplyBookMetadataMutation = {
	applyBookMetadata: {
		mediaId: string
		workId: string | null
		title: string
		authors: Array<string>
		workMetadata: {
			workId: string
			title: string | null
			author: string | null
			metadata: unknown
			lockedFields: Array<MetadataField>
		} | null
		editions: Array<{
			mediaId: string
			kind: BookEditionKind
			title: string
			pairEvidence: PairEvidence | null
			metadata: {
				id: number
				title: string | null
				titleSort: string | null
				series: string | null
				seriesGroup: string | null
				storyArc: string | null
				storyArcNumber: unknown
				number: unknown
				volume: number | null
				summary: string | null
				notes: string | null
				genres: Array<string>
				format: string | null
				year: number | null
				month: number | null
				day: number | null
				writers: Array<string>
				pencillers: Array<string>
				inkers: Array<string>
				colorists: Array<string>
				letterers: Array<string>
				coverArtists: Array<string>
				editors: Array<string>
				narrators: Array<string>
				publisher: string | null
				links: Array<string>
				characters: Array<string>
				teams: Array<string>
				pageCount: number | null
				ageRating: number | null
				identifierAmazon: string | null
				identifierCalibre: string | null
				identifierGoogle: string | null
				identifierIsbn: string | null
				identifierMobiAsin: string | null
				identifierUuid: string | null
				language: string | null
				metadataSource: string | null
				metadataExternalId: string | null
				lockedFields: Array<MetadataField>
			} | null
			file: {
				path: string
				size: number
				extension: string
				hash: string | null
				koreaderHash: string | null
				status: FileStatus
				modifiedAt: string | null
			}
			audio: {
				durationMs: number
				codec: string
				sampleRate: number | null
				channels: number | null
				bitrate: number | null
				chapterSource: string
				chapters: Array<{
					id: string
					index: number
					title: string | null
					startMs: number
					endMs: number | null
				}>
			} | null
		}>
		mismatches: Array<{
			field: MetadataField
			ebookValue: string | null
			audiobookValue: string | null
			workValue: string | null
			resolved: boolean
		}>
		review: {
			id: string
			rating: number
			content: string | null
			isPrivate: boolean
			mediaId: string | null
			workId: string | null
			createdAt: string
			updatedAt: string
		} | null
		readAloud: {
			status: BookReadAloudStatus
			reason: string | null
			ebookMediaId: string | null
			audioMediaId: string | null
			chapterMap: Array<{
				ebookMediaId: string
				audioMediaId: string
				ebookSpineIndex: number
				audioChapterIndex: number
				confidence: number
			}>
			syncMap: {
				id: string
				ebookMediaId: string
				audioMediaId: string
				granularity: AlignGranularity
				generator: string
				generatorVersion: string
				algorithm: string | null
				model: string | null
				cueCount: number
				coverage: number | null
				source: string
				jobId: string | null
				createdAt: string
				map: unknown
			} | null
			artifact: { url: string; mimeType: string; cacheKey: string } | null
		}
	}
}

export type ApplyBookMetadataCandidateMutationVariables = Exact<{
	mediaId: string | number
	candidateIndex: number
	scope: BookMetadataScope
	selectedFields: Array<MetadataField> | MetadataField
}>

export type ApplyBookMetadataCandidateMutation = {
	applyBookMetadataCandidate: {
		mediaId: string
		workId: string | null
		title: string
		authors: Array<string>
		workMetadata: {
			workId: string
			title: string | null
			author: string | null
			metadata: unknown
			lockedFields: Array<MetadataField>
		} | null
		editions: Array<{
			mediaId: string
			kind: BookEditionKind
			title: string
			pairEvidence: PairEvidence | null
			metadata: {
				id: number
				title: string | null
				titleSort: string | null
				series: string | null
				seriesGroup: string | null
				storyArc: string | null
				storyArcNumber: unknown
				number: unknown
				volume: number | null
				summary: string | null
				notes: string | null
				genres: Array<string>
				format: string | null
				year: number | null
				month: number | null
				day: number | null
				writers: Array<string>
				pencillers: Array<string>
				inkers: Array<string>
				colorists: Array<string>
				letterers: Array<string>
				coverArtists: Array<string>
				editors: Array<string>
				narrators: Array<string>
				publisher: string | null
				links: Array<string>
				characters: Array<string>
				teams: Array<string>
				pageCount: number | null
				ageRating: number | null
				identifierAmazon: string | null
				identifierCalibre: string | null
				identifierGoogle: string | null
				identifierIsbn: string | null
				identifierMobiAsin: string | null
				identifierUuid: string | null
				language: string | null
				metadataSource: string | null
				metadataExternalId: string | null
				lockedFields: Array<MetadataField>
			} | null
			file: {
				path: string
				size: number
				extension: string
				hash: string | null
				koreaderHash: string | null
				status: FileStatus
				modifiedAt: string | null
			}
			audio: {
				durationMs: number
				codec: string
				sampleRate: number | null
				channels: number | null
				bitrate: number | null
				chapterSource: string
				chapters: Array<{
					id: string
					index: number
					title: string | null
					startMs: number
					endMs: number | null
				}>
			} | null
		}>
		mismatches: Array<{
			field: MetadataField
			ebookValue: string | null
			audiobookValue: string | null
			workValue: string | null
			resolved: boolean
		}>
		review: {
			id: string
			rating: number
			content: string | null
			isPrivate: boolean
			mediaId: string | null
			workId: string | null
			createdAt: string
			updatedAt: string
		} | null
		readAloud: {
			status: BookReadAloudStatus
			reason: string | null
			ebookMediaId: string | null
			audioMediaId: string | null
			chapterMap: Array<{
				ebookMediaId: string
				audioMediaId: string
				ebookSpineIndex: number
				audioChapterIndex: number
				confidence: number
			}>
			syncMap: {
				id: string
				ebookMediaId: string
				audioMediaId: string
				granularity: AlignGranularity
				generator: string
				generatorVersion: string
				algorithm: string | null
				model: string | null
				cueCount: number
				coverage: number | null
				source: string
				jobId: string | null
				createdAt: string
				map: unknown
			} | null
			artifact: { url: string; mimeType: string; cacheKey: string } | null
		}
	}
}

export type UpsertBookReviewMutationVariables = Exact<{
	mediaId: string | number
	input: BookReviewInput
}>

export type UpsertBookReviewMutation = {
	upsertBookReview: {
		id: string
		rating: number
		content: string | null
		isPrivate: boolean
		mediaId: string | null
		workId: string | null
		createdAt: string
		updatedAt: string
	}
}

export type DeleteBookReviewMutationVariables = Exact<{
	mediaId: string | number
}>

export type DeleteBookReviewMutation = { deleteBookReview: boolean }

export type BookDetailFieldsFragment = {
	mediaId: string
	workId: string | null
	title: string
	authors: Array<string>
	workMetadata: {
		workId: string
		title: string | null
		author: string | null
		metadata: unknown
		lockedFields: Array<MetadataField>
	} | null
	editions: Array<{
		mediaId: string
		kind: BookEditionKind
		title: string
		pairEvidence: PairEvidence | null
		metadata: {
			id: number
			title: string | null
			titleSort: string | null
			series: string | null
			seriesGroup: string | null
			storyArc: string | null
			storyArcNumber: unknown
			number: unknown
			volume: number | null
			summary: string | null
			notes: string | null
			genres: Array<string>
			format: string | null
			year: number | null
			month: number | null
			day: number | null
			writers: Array<string>
			pencillers: Array<string>
			inkers: Array<string>
			colorists: Array<string>
			letterers: Array<string>
			coverArtists: Array<string>
			editors: Array<string>
			narrators: Array<string>
			publisher: string | null
			links: Array<string>
			characters: Array<string>
			teams: Array<string>
			pageCount: number | null
			ageRating: number | null
			identifierAmazon: string | null
			identifierCalibre: string | null
			identifierGoogle: string | null
			identifierIsbn: string | null
			identifierMobiAsin: string | null
			identifierUuid: string | null
			language: string | null
			metadataSource: string | null
			metadataExternalId: string | null
			lockedFields: Array<MetadataField>
		} | null
		file: {
			path: string
			size: number
			extension: string
			hash: string | null
			koreaderHash: string | null
			status: FileStatus
			modifiedAt: string | null
		}
		audio: {
			durationMs: number
			codec: string
			sampleRate: number | null
			channels: number | null
			bitrate: number | null
			chapterSource: string
			chapters: Array<{
				id: string
				index: number
				title: string | null
				startMs: number
				endMs: number | null
			}>
		} | null
	}>
	mismatches: Array<{
		field: MetadataField
		ebookValue: string | null
		audiobookValue: string | null
		workValue: string | null
		resolved: boolean
	}>
	review: {
		id: string
		rating: number
		content: string | null
		isPrivate: boolean
		mediaId: string | null
		workId: string | null
		createdAt: string
		updatedAt: string
	} | null
	readAloud: {
		status: BookReadAloudStatus
		reason: string | null
		ebookMediaId: string | null
		audioMediaId: string | null
		chapterMap: Array<{
			ebookMediaId: string
			audioMediaId: string
			ebookSpineIndex: number
			audioChapterIndex: number
			confidence: number
		}>
		syncMap: {
			id: string
			ebookMediaId: string
			audioMediaId: string
			granularity: AlignGranularity
			generator: string
			generatorVersion: string
			algorithm: string | null
			model: string | null
			cueCount: number
			coverage: number | null
			source: string
			jobId: string | null
			createdAt: string
			map: unknown
		} | null
		artifact: { url: string; mimeType: string; cacheKey: string } | null
	}
}

export type ConnectionKindleDestinationFieldsFragment = {
	id: string
	name: string
	email: string
	isDefault: boolean
	createdAt: string
	updatedAt: string
}

export type ConnectionKindleDestinationDeliveryFieldsFragment = {
	id: string
	mediaId: string
	destinationId: string
	destinationName: string
	destinationEmail: string
	recipient: string
	format: string
	bytes: number
	converted: boolean
	note: string | null
	error: string | null
	sentAt: string
}

export type ConnectionKindleDestinationsQueryVariables = Exact<{ [key: string]: never }>

export type ConnectionKindleDestinationsQuery = {
	kindleDestinations: Array<{
		id: string
		name: string
		email: string
		isDefault: boolean
		createdAt: string
		updatedAt: string
	}>
}

export type ConnectionUpsertKindleDestinationMutationVariables = Exact<{
	id?: string | number | null | undefined
	name: string
	email: string
	makeDefault: boolean
}>

export type ConnectionUpsertKindleDestinationMutation = {
	upsertKindleDestination: {
		id: string
		name: string
		email: string
		isDefault: boolean
		createdAt: string
		updatedAt: string
	}
}

export type ConnectionDeleteKindleDestinationMutationVariables = Exact<{
	id: string | number
}>

export type ConnectionDeleteKindleDestinationMutation = { deleteKindleDestination: boolean }

export type ConnectionKindleDestinationDeliveriesQueryVariables = Exact<{
	mediaId?: string | number | null | undefined
	limit?: number | null | undefined
}>

export type ConnectionKindleDestinationDeliveriesQuery = {
	kindleDestinationDeliveries: Array<{
		id: string
		mediaId: string
		destinationId: string
		destinationName: string
		destinationEmail: string
		recipient: string
		format: string
		bytes: number
		converted: boolean
		note: string | null
		error: string | null
		sentAt: string
	}>
}

export type ConnectionSendToKindleDestinationMutationVariables = Exact<{
	mediaId: string | number
	destinationId: string | number
}>

export type ConnectionSendToKindleDestinationMutation = {
	sendToKindleDestination: {
		id: string
		mediaId: string
		destinationId: string
		destinationName: string
		destinationEmail: string
		recipient: string
		format: string
		bytes: number
		converted: boolean
		note: string | null
		error: string | null
		sentAt: string
	}
}

export type ConnectionHardcoverFieldsFragment = {
	connected: boolean
	remoteUserId: string | null
	remoteUsername: string | null
	scopes: Array<string>
	capabilities: Array<string>
	useForMetadata: boolean
	importJournals: boolean
	syncProgress: boolean
	connectedAt: string | null
	verifiedAt: string | null
	lastSyncAt: string | null
	lastError: string | null
}

export type ConnectionHardcoverQueryVariables = Exact<{ [key: string]: never }>

export type ConnectionHardcoverQuery = {
	hardcoverConnection: {
		connected: boolean
		remoteUserId: string | null
		remoteUsername: string | null
		scopes: Array<string>
		capabilities: Array<string>
		useForMetadata: boolean
		importJournals: boolean
		syncProgress: boolean
		connectedAt: string | null
		verifiedAt: string | null
		lastSyncAt: string | null
		lastError: string | null
	} | null
}

export type ConnectionConnectHardcoverMutationVariables = Exact<{
	apiToken: string
	useForMetadata?: boolean | null | undefined
	importJournals?: boolean | null | undefined
	syncProgress?: boolean | null | undefined
}>

export type ConnectionConnectHardcoverMutation = {
	connectHardcover: {
		connected: boolean
		remoteUserId: string | null
		remoteUsername: string | null
		scopes: Array<string>
		capabilities: Array<string>
		useForMetadata: boolean
		importJournals: boolean
		syncProgress: boolean
		connectedAt: string | null
		verifiedAt: string | null
		lastSyncAt: string | null
		lastError: string | null
	}
}

export type ConnectionUpdateHardcoverMutationVariables = Exact<{
	useForMetadata: boolean
	importJournals: boolean
	syncProgress: boolean
}>

export type ConnectionUpdateHardcoverMutation = {
	updateHardcoverConnection: {
		connected: boolean
		remoteUserId: string | null
		remoteUsername: string | null
		scopes: Array<string>
		capabilities: Array<string>
		useForMetadata: boolean
		importJournals: boolean
		syncProgress: boolean
		connectedAt: string | null
		verifiedAt: string | null
		lastSyncAt: string | null
		lastError: string | null
	}
}

export type ConnectionDisconnectHardcoverMutationVariables = Exact<{ [key: string]: never }>

export type ConnectionDisconnectHardcoverMutation = { disconnectHardcover: boolean }

export type ConnectionAdminEmailersQueryVariables = Exact<{ [key: string]: never }>

export type ConnectionAdminEmailersQuery = {
	emailers: Array<{
		id: number
		name: string
		isPrimary: boolean
		senderEmail: string
		senderDisplayName: string
		username: string
		smtpHost: string
		smtpPort: number
		tlsEnabled: boolean
		lastUsedAt: string | null
	}>
}

export type ConnectionCreateAdminEmailerMutationVariables = Exact<{
	input: EmailerInput
}>

export type ConnectionCreateAdminEmailerMutation = {
	createEmailer: {
		id: number
		name: string
		isPrimary: boolean
		senderEmail: string
		senderDisplayName: string
		username: string
		smtpHost: string
		smtpPort: number
		tlsEnabled: boolean
		lastUsedAt: string | null
	}
}

export type ConnectionUpdateAdminEmailerMutationVariables = Exact<{
	id: number
	input: EmailerInput
}>

export type ConnectionUpdateAdminEmailerMutation = {
	updateEmailer: {
		id: number
		name: string
		isPrimary: boolean
		senderEmail: string
		senderDisplayName: string
		username: string
		smtpHost: string
		smtpPort: number
		tlsEnabled: boolean
		lastUsedAt: string | null
	}
}

export type ConnectionTestAdminEmailerMutationVariables = Exact<{
	config: EmailerClientConfig
	recipient: string
}>

export type ConnectionTestAdminEmailerMutation = { testEmailer: boolean }

export type ConnectionHardcoverLinkFieldsFragment = {
	id: string
	mediaId: string
	remoteId: string
	remoteTitle: string | null
	linkedAt: string
	updatedAt: string
}

export type ConnectionHardcoverLinksQueryVariables = Exact<{
	mediaId?: string | number | null | undefined
}>

export type ConnectionHardcoverLinksQuery = {
	hardcoverMediaLinks: Array<{
		id: string
		mediaId: string
		remoteId: string
		remoteTitle: string | null
		linkedAt: string
		updatedAt: string
	}>
}

export type ConnectionLinkHardcoverMediaMutationVariables = Exact<{
	mediaId: string | number
	remoteId: string
}>

export type ConnectionLinkHardcoverMediaMutation = {
	linkHardcoverMedia: {
		id: string
		mediaId: string
		remoteId: string
		remoteTitle: string | null
		linkedAt: string
		updatedAt: string
	}
}

export type ConnectionUnlinkHardcoverMediaMutationVariables = Exact<{
	mediaId: string | number
}>

export type ConnectionUnlinkHardcoverMediaMutation = { unlinkHardcoverMedia: boolean }

export type ConnectionSmtpFieldsFragment = {
	configured: boolean
	senderEmail: string | null
	senderDisplayName: string | null
	smtpHost: string | null
	smtpPort: number | null
	tlsEnabled: boolean | null
	lastUsedAt: string | null
}

export type ConnectionSmtpQueryVariables = Exact<{ [key: string]: never }>

export type ConnectionSmtpQuery = {
	smtpSettings: {
		configured: boolean
		senderEmail: string | null
		senderDisplayName: string | null
		smtpHost: string | null
		smtpPort: number | null
		tlsEnabled: boolean | null
		lastUsedAt: string | null
	}
}

export type ConnectionAdminBoundaryQueryVariables = Exact<{ [key: string]: never }>

export type ConnectionAdminBoundaryQuery = { annotationSyncRoot: string | null }

export type ConnectionSyncHardcoverNowMutationVariables = Exact<{ [key: string]: never }>

export type ConnectionSyncHardcoverNowMutation = {
	syncHardcoverNow: {
		status: string
		imported: number
		unresolved: number
		projected: number
		skipped: number
		lastSyncAt: string | null
		error: string | null
	}
}

export type CrosspointTargetFieldsFragment = {
	deviceId: string
	userId: string
	hostOrIp: string
	httpPort: number
	wsPort: number
	rootPath: string
	discoveryMethod: string
	verifiedAt: string | null
	revokedAt: string | null
	profileJson: unknown
	profileDigest: string
	fingerprint: { model: string; serial: string } | null
	profile: {
		optimizerEnabled: boolean
		targetModel: CrosspointTargetModel
		jpegQuality: number
		grayscale: boolean
		autoCrop: boolean
		splitLargeParagraphs: boolean
		removeFonts: boolean
		chunkBytes: number
		retryCount: number
		retryDelaySeconds: number
		timeoutSeconds: number
		maxUploadBytes: number
	}
}

export type CrosspointTargetVerificationFieldsFragment = {
	hostOrIp: string
	httpPort: number
	wsPort: number
	model: string
	serial: string
	verified: boolean
}

export type CrosspointDeliveryFieldsFragment = {
	id: string
	userId: string
	deviceId: string
	mediaId: string
	sourceRevision: string
	profileDigest: string
	profileJson: string
	destinationPath: string
	idempotencyKey: string
	status: CrosspointDeliveryStatus
	attempts: number
	maxAttempts: number
	nextAttemptAt: string | null
	lastError: string | null
	queuedAt: string
	startedAt: string | null
	completedAt: string | null
}

export type CrosspointTargetQueryVariables = Exact<{
	deviceId: string | number
}>

export type CrosspointTargetQuery = {
	crosspointTarget: {
		deviceId: string
		userId: string
		hostOrIp: string
		httpPort: number
		wsPort: number
		rootPath: string
		discoveryMethod: string
		verifiedAt: string | null
		revokedAt: string | null
		profileJson: unknown
		profileDigest: string
		fingerprint: { model: string; serial: string } | null
		profile: {
			optimizerEnabled: boolean
			targetModel: CrosspointTargetModel
			jpegQuality: number
			grayscale: boolean
			autoCrop: boolean
			splitLargeParagraphs: boolean
			removeFonts: boolean
			chunkBytes: number
			retryCount: number
			retryDelaySeconds: number
			timeoutSeconds: number
			maxUploadBytes: number
		}
	} | null
}

export type VerifyCrosspointTargetMutationVariables = Exact<{
	deviceId: string | number
	host: string
}>

export type VerifyCrosspointTargetMutation = {
	verifyCrosspointTarget: {
		hostOrIp: string
		httpPort: number
		wsPort: number
		model: string
		serial: string
		verified: boolean
	}
}

export type UpdateCrosspointTargetMutationVariables = Exact<{
	deviceId: string | number
	input: CrosspointTargetInput
}>

export type UpdateCrosspointTargetMutation = {
	updateCrosspointTarget: {
		deviceId: string
		userId: string
		hostOrIp: string
		httpPort: number
		wsPort: number
		rootPath: string
		discoveryMethod: string
		verifiedAt: string | null
		revokedAt: string | null
		profileJson: unknown
		profileDigest: string
		fingerprint: { model: string; serial: string } | null
		profile: {
			optimizerEnabled: boolean
			targetModel: CrosspointTargetModel
			jpegQuality: number
			grayscale: boolean
			autoCrop: boolean
			splitLargeParagraphs: boolean
			removeFonts: boolean
			chunkBytes: number
			retryCount: number
			retryDelaySeconds: number
			timeoutSeconds: number
			maxUploadBytes: number
		}
	}
}

export type CrosspointDeliveriesQueryVariables = Exact<{
	deviceId: string | number
	limit?: number | null | undefined
}>

export type CrosspointDeliveriesQuery = {
	crosspointDeliveries: Array<{
		id: string
		userId: string
		deviceId: string
		mediaId: string
		sourceRevision: string
		profileDigest: string
		profileJson: string
		destinationPath: string
		idempotencyKey: string
		status: CrosspointDeliveryStatus
		attempts: number
		maxAttempts: number
		nextAttemptAt: string | null
		lastError: string | null
		queuedAt: string
		startedAt: string | null
		completedAt: string | null
	}>
}

export type QueueCrosspointDeliveriesMutationVariables = Exact<{
	deviceId: string | number
	mediaIds: Array<string | number> | string | number
	targetPath: string
}>

export type QueueCrosspointDeliveriesMutation = {
	queueCrosspointDeliveries: Array<{
		id: string
		userId: string
		deviceId: string
		mediaId: string
		sourceRevision: string
		profileDigest: string
		profileJson: string
		destinationPath: string
		idempotencyKey: string
		status: CrosspointDeliveryStatus
		attempts: number
		maxAttempts: number
		nextAttemptAt: string | null
		lastError: string | null
		queuedAt: string
		startedAt: string | null
		completedAt: string | null
	}>
}

export type RetryCrosspointDeliveryMutationVariables = Exact<{
	id: string | number
}>

export type RetryCrosspointDeliveryMutation = {
	retryCrosspointDelivery: {
		id: string
		userId: string
		deviceId: string
		mediaId: string
		sourceRevision: string
		profileDigest: string
		profileJson: string
		destinationPath: string
		idempotencyKey: string
		status: CrosspointDeliveryStatus
		attempts: number
		maxAttempts: number
		nextAttemptAt: string | null
		lastError: string | null
		queuedAt: string
		startedAt: string | null
		completedAt: string | null
	}
}

export type CancelCrosspointDeliveryMutationVariables = Exact<{
	id: string | number
}>

export type CancelCrosspointDeliveryMutation = {
	cancelCrosspointDelivery: {
		id: string
		userId: string
		deviceId: string
		mediaId: string
		sourceRevision: string
		profileDigest: string
		profileJson: string
		destinationPath: string
		idempotencyKey: string
		status: CrosspointDeliveryStatus
		attempts: number
		maxAttempts: number
		nextAttemptAt: string | null
		lastError: string | null
		queuedAt: string
		startedAt: string | null
		completedAt: string | null
	}
}

export type DashboardViewerQueryVariables = Exact<{ [key: string]: never }>

export type DashboardViewerQuery = {
	me: { id: string; username: string; isServerOwner: boolean; permissions: Array<UserPermission> }
}

export type DashboardBookCardFragment = {
	id: string
	resolvedName: string
	extension: string
	pages: number
	createdAt: string
	seriesId: string | null
	thumbnail: { url: string }
	series: { id: string; resolvedName: string }
	audio: { durationMs: number } | null
	readProgress: {
		page: number | null
		positionMs: number | null
		percentageCompleted: unknown
		elapsedSeconds: number
		updatedAt: string | null
	} | null
}

export type DashboardKeepReadingQueryVariables = Exact<{
	pagination: Pagination
}>

export type DashboardKeepReadingQuery = {
	keepReading: {
		nodes: Array<{
			id: string
			resolvedName: string
			extension: string
			pages: number
			createdAt: string
			seriesId: string | null
			thumbnail: { url: string }
			series: { id: string; resolvedName: string }
			audio: { durationMs: number } | null
			readProgress: {
				page: number | null
				positionMs: number | null
				percentageCompleted: unknown
				elapsedSeconds: number
				updatedAt: string | null
			} | null
		}>
	}
}

export type DashboardRecentlyAddedQueryVariables = Exact<{
	pagination: Pagination
}>

export type DashboardRecentlyAddedQuery = {
	recentlyAddedMedia: {
		nodes: Array<{
			id: string
			resolvedName: string
			extension: string
			pages: number
			createdAt: string
			seriesId: string | null
			thumbnail: { url: string }
			series: { id: string; resolvedName: string }
			audio: { durationMs: number } | null
			readProgress: {
				page: number | null
				positionMs: number | null
				percentageCompleted: unknown
				elapsedSeconds: number
				updatedAt: string | null
			} | null
		}>
	}
}

export type DashboardJobsQueryVariables = Exact<{
	pagination: Pagination
}>

export type DashboardJobsQuery = {
	jobs: {
		nodes: Array<{
			id: string
			name: string
			description: string | null
			status: JobStatus
			msElapsed: number
			createdAt: string
			completedAt: string | null
		}>
	}
}

export type DashboardRecentAnnotationsQueryVariables = Exact<{
	pageSize: number
}>

export type DashboardRecentAnnotationsQuery = {
	annotations: {
		total: number
		items: Array<{
			id: string
			kind: AnnotationKind
			source: DeviceKind
			sourceDeviceName: string | null
			excerpt: string | null
			note: string | null
			chapterTitle: string | null
			page: number | null
			createdAt: string | null
			book: { key: string; mediaId: string | null; title: string }
		}>
	}
}

export type AccountProfileQueryVariables = Exact<{ [key: string]: never }>

export type AccountProfileQuery = {
	me: {
		id: string
		username: string
		isServerOwner: boolean
		oidcEmail: string | null
		createdAt: string
		lastLogin: string | null
		loginSessionsCount: number
		maxSessionsAllowed: number | null
		finishedReadingSessionsCount: number
		permissions: Array<UserPermission>
		avatar: { url: string }
		preferences: { appTheme: string; locale: string }
	}
}

export type AccountSignOutEverywhereMutationVariables = Exact<{
	id: string | number
}>

export type AccountSignOutEverywhereMutation = { deleteUserSessions: number }

export type DashboardLiveEventsSubscriptionVariables = Exact<{ [key: string]: never }>

export type DashboardLiveEventsSubscription = {
	readEvents:
		| { __typename: 'AnalysisJobFailed' }
		| { __typename: 'CollectionAdded' }
		| { __typename: 'CollectionChanged' }
		| { __typename: 'CollectionDeleted' }
		| { __typename: 'CreatedManySeries' }
		| { __typename: 'CreatedMedia' }
		| { __typename: 'CreatedOrUpdatedManyMedia' }
		| { __typename: 'DevicePaired' }
		| { __typename: 'DevicePairingRequested' }
		| { __typename: 'DeviceSeen'; deviceId: string; protocol: DeviceProtocol }
		| { __typename: 'DiscoveredMissingLibrary' }
		| { __typename: 'IngestAwaitingReview' }
		| { __typename: 'IngestItemChanged' }
		| { __typename: 'JobOutput'; id: string }
		| { __typename: 'JobQueueStatus'; count: number; countByType: unknown }
		| { __typename: 'JobStarted'; id: string }
		| {
				__typename: 'JobUpdate'
				id: string
				status: JobStatus | null
				message: string | null
				subtitle: string | null
				completedTasks: number | null
				remainingTasks: number | null
		  }
		| { __typename: 'LibraryCreated' }
		| { __typename: 'LibraryDeleted' }
		| { __typename: 'LibraryUpdated' }
		| { __typename: 'MediaDeleted' }
		| { __typename: 'ProviderCatalogRefreshed' }
		| { __typename: 'ProviderMatchDone' }
		| { __typename: 'ProviderSeriesMaterialized' }
		| {
				__typename: 'ProviderSourceHealthChanged'
				sourceId: string
				name: string
				healthStatus: string
		  }
		| { __typename: 'QualityFailed' }
		| { __typename: 'ReadListAdded' }
		| { __typename: 'ReadListChanged' }
		| { __typename: 'ReadListDeleted' }
		| { __typename: 'SeriesDeleted' }
		| { __typename: 'WorkerJobChanged' }
}

export type NotificationSettingsQueryVariables = Exact<{ [key: string]: never }>

export type NotificationSettingsQuery = {
	notificationChannels: Array<{
		id: string
		label: string
		settings: Array<{
			key: string
			label: string
			valueType: IngestSettingValueType
			required: boolean
			secret: boolean
			defaultValue: unknown
			description: string | null
			helpUrl: string | null
		}>
	}>
	notificationRules: Array<{ id: number; eventKind: string; channelId: string; enabled: boolean }>
	notificationChannelSettings: Array<{ channelId: string; values: unknown }>
}

export type SetNotificationRuleMutationVariables = Exact<{
	eventKind: NotificationKind
	channelId: string
	enabled: boolean
}>

export type SetNotificationRuleMutation = {
	setNotificationRule: { id: number; eventKind: string; channelId: string; enabled: boolean }
}

export type SetNotificationChannelSettingsMutationVariables = Exact<{
	input: SetNotificationChannelSettingsInput
}>

export type SetNotificationChannelSettingsMutation = {
	setNotificationChannelSettings: { channelId: string; values: unknown }
}

export type TestNotificationChannelMutationVariables = Exact<{
	channelId: string
}>

export type TestNotificationChannelMutation = { testNotificationChannel: boolean }

export type ConsolePageInfoFragment = {
	totalItems: number
	totalPages: number
	currentPage: number
	pageSize: number
}

export type ConsoleLibraryCardFragment = {
	id: string
	name: string
	description: string | null
	path: string
	emoji: string | null
	status: FileStatus
	lastScannedAt: string | null
	sourceProvider: string | null
	config: { libraryType: LibraryType; libraryPattern: LibraryPattern; watch: boolean }
	stats: {
		seriesCount: number
		bookCount: number
		completedBooks: number
		inProgressBooks: number
		totalBytes: number
	}
}

export type ConsoleSeriesCardFragment = {
	id: string
	name: string
	resolvedName: string
	path: string
	status: FileStatus
	mediaCount: number
	readCount: number
	unreadCount: number
	percentageCompleted: number
	isComplete: boolean
	sourceProvider: string | null
	thumbnail: { url: string }
	metadata: { title: string | null; publisher: string | null } | null
	tags: Array<{ id: number; name: string }>
}

export type ConsoleBookRowFragment = {
	id: string
	name: string
	resolvedName: string
	extension: string
	pages: number
	size: number
	status: FileStatus
	seriesId: string | null
	path: string
	series: { id: string; resolvedName: string }
	thumbnail: { url: string }
	metadata: { title: string | null; publisher: string | null; writers: Array<string> } | null
	tags: Array<{ id: number; name: string }>
	readProgress: {
		page: number | null
		percentageCompleted: unknown
		updatedAt: string | null
	} | null
	readHistory: Array<{ readthroughNumber: number; completedAt: string; dnf: boolean }>
}

export type ConsoleLibrariesQueryVariables = Exact<{
	pagination: Pagination
}>

export type ConsoleLibrariesQuery = {
	libraries: {
		nodes: Array<{
			id: string
			name: string
			description: string | null
			path: string
			emoji: string | null
			status: FileStatus
			lastScannedAt: string | null
			sourceProvider: string | null
			config: { libraryType: LibraryType; libraryPattern: LibraryPattern; watch: boolean }
			stats: {
				seriesCount: number
				bookCount: number
				completedBooks: number
				inProgressBooks: number
				totalBytes: number
			}
		}>
		pageInfo:
			| { totalItems: number; totalPages: number; currentPage: number; pageSize: number }
			| Record<PropertyKey, never>
	}
}

export type ConsoleLibraryOptionsQueryVariables = Exact<{ [key: string]: never }>

export type ConsoleLibraryOptionsQuery = {
	libraries: { nodes: Array<{ id: string; name: string; emoji: string | null }> }
}

export type ConsoleLibraryDetailQueryVariables = Exact<{
	id: string | number
}>

export type ConsoleLibraryDetailQuery = {
	libraryById: {
		id: string
		name: string
		description: string | null
		path: string
		emoji: string | null
		status: FileStatus
		lastScannedAt: string | null
		sourceProvider: string | null
		tags: Array<{ id: number; name: string }>
		config: { libraryType: LibraryType; libraryPattern: LibraryPattern; watch: boolean }
		stats: {
			seriesCount: number
			bookCount: number
			completedBooks: number
			inProgressBooks: number
			totalBytes: number
		}
	} | null
}

export type ConsoleLibrarySeriesQueryVariables = Exact<{
	filter: SeriesFilterInput
	orderBy: Array<SeriesOrderBy> | SeriesOrderBy
	pagination: Pagination
}>

export type ConsoleLibrarySeriesQuery = {
	series: {
		nodes: Array<{
			id: string
			name: string
			resolvedName: string
			path: string
			status: FileStatus
			mediaCount: number
			readCount: number
			unreadCount: number
			percentageCompleted: number
			isComplete: boolean
			sourceProvider: string | null
			thumbnail: { url: string }
			metadata: { title: string | null; publisher: string | null } | null
			tags: Array<{ id: number; name: string }>
		}>
		pageInfo:
			| { totalItems: number; totalPages: number; currentPage: number; pageSize: number }
			| Record<PropertyKey, never>
	}
}

export type ConsoleBooksQueryVariables = Exact<{
	filter: MediaFilterInput
	orderBy: Array<MediaOrderBy> | MediaOrderBy
	pagination: Pagination
}>

export type ConsoleBooksQuery = {
	media: {
		nodes: Array<{
			id: string
			name: string
			resolvedName: string
			extension: string
			pages: number
			size: number
			status: FileStatus
			seriesId: string | null
			path: string
			series: { id: string; resolvedName: string }
			thumbnail: { url: string }
			metadata: { title: string | null; publisher: string | null; writers: Array<string> } | null
			tags: Array<{ id: number; name: string }>
			readProgress: {
				page: number | null
				percentageCompleted: unknown
				updatedAt: string | null
			} | null
			readHistory: Array<{ readthroughNumber: number; completedAt: string; dnf: boolean }>
		}>
		pageInfo:
			| { totalItems: number; totalPages: number; currentPage: number; pageSize: number }
			| Record<PropertyKey, never>
	}
}

export type ConsoleSeriesDetailQueryVariables = Exact<{
	id: string | number
}>

export type ConsoleSeriesDetailQuery = {
	seriesById: {
		description: string | null
		resolvedDescription: string | null
		libraryId: string | null
		id: string
		name: string
		resolvedName: string
		path: string
		status: FileStatus
		mediaCount: number
		readCount: number
		unreadCount: number
		percentageCompleted: number
		isComplete: boolean
		sourceProvider: string | null
		library: { id: string; name: string; config: { libraryType: LibraryType } }
		stats: {
			bookCount: number
			completedBooks: number
			inProgressBooks: number
			totalReadingTimeSeconds: number
		}
		metadata: {
			ageRating: number | null
			booktype: string | null
			characters: Array<string>
			comicImage: string | null
			comicid: number | null
			descriptionFormatted: string | null
			genres: Array<string>
			imprint: string | null
			links: Array<string>
			metaType: string | null
			publicationRun: string | null
			publisher: string | null
			status: string | null
			summary: string | null
			title: string | null
			totalIssues: number | null
			volume: number | null
			writers: Array<string>
			year: number | null
			collects: Array<{
				series: string | null
				comicid: string | null
				issueid: string | null
				issues: string | null
			}>
		} | null
		thumbnail: { url: string }
		tags: Array<{ id: number; name: string }>
	} | null
}

export type ConsoleSeriesPickerQueryVariables = Exact<{
	filter: SeriesFilterInput
}>

export type ConsoleSeriesPickerQuery = {
	series: {
		nodes: Array<{
			id: string
			name: string
			resolvedName: string
			mediaCount: number
			sourceProvider: string | null
		}>
	}
}

export type ConsoleAuthorsQueryVariables = Exact<{
	search?: string | null | undefined
	libraryId?: string | null | undefined
	pagination: Pagination
}>

export type ConsoleAuthorsQuery = {
	authors: {
		nodes: Array<{ name: string; books: Array<{ id: string }>; series: Array<{ title: string }> }>
		pageInfo:
			| { totalItems: number; totalPages: number; currentPage: number; pageSize: number }
			| Record<PropertyKey, never>
	}
}

export type ConsoleLibraryPublishersQueryVariables = Exact<{
	id: string | number
}>

export type ConsoleLibraryPublishersQuery = {
	libraryById: { id: string; publishers: Array<string> } | null
}

export type ConsoleTagsQueryVariables = Exact<{ [key: string]: never }>

export type ConsoleTagsQuery = { tags: Array<{ id: number; name: string; kind: string }> }

export type ConsoleEntityBookCountQueryVariables = Exact<{
	filter: MediaFilterInput
}>

export type ConsoleEntityBookCountQuery = {
	media: {
		pageInfo:
			| { totalItems: number; totalPages: number; currentPage: number; pageSize: number }
			| Record<PropertyKey, never>
	}
}

export type ConsoleCreateLibraryMutationVariables = Exact<{
	input: CreateOrUpdateLibraryInput
}>

export type ConsoleCreateLibraryMutation = {
	createLibrary: {
		id: string
		name: string
		description: string | null
		path: string
		emoji: string | null
		status: FileStatus
		lastScannedAt: string | null
		sourceProvider: string | null
		config: { libraryType: LibraryType; libraryPattern: LibraryPattern; watch: boolean }
		stats: {
			seriesCount: number
			bookCount: number
			completedBooks: number
			inProgressBooks: number
			totalBytes: number
		}
	}
}

export type ConsoleScanLibraryMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleScanLibraryMutation = { scanLibrary: boolean }

export type ConsoleAnalyzeLibraryMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleAnalyzeLibraryMutation = { analyzeLibrary: boolean }

export type ConsoleFinishMediaMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleFinishMediaMutation = { finishMediaProgress: boolean }

export type ConsoleResetMediaProgressMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleResetMediaProgressMutation = {
	clearMediaProgress: boolean
	deleteMediaReadingHistory: number
}

export type ConsoleFinishSeriesMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleFinishSeriesMutation = { finishSeriesProgress: number }

export type ConsoleClearSeriesHistoryMutationVariables = Exact<{
	id: string | number
}>

export type ConsoleClearSeriesHistoryMutation = { clearSeriesReadingHistory: number }

export type ConsoleRenameSeriesMutationVariables = Exact<{
	id: string | number
	input: SeriesMetadataInput
}>

export type ConsoleRenameSeriesMutation = {
	updateSeriesMetadata: {
		id: string
		name: string
		resolvedName: string
		metadata: { title: string | null } | null
	}
}

export type ConsoleMoveMediaToSeriesMutationVariables = Exact<{
	mediaIds: Array<string | number> | string | number
	seriesId: string | number
}>

export type ConsoleMoveMediaToSeriesMutation = {
	moveMediaToSeries: { id: string; name: string; resolvedName: string; mediaCount: number }
}

export type ConsoleMergeSeriesMutationVariables = Exact<{
	keep: string | number
	drop: string | number
}>

export type ConsoleMergeSeriesMutation = {
	mergeSeries: {
		droppedSeriesId: string
		moved: number
		missingFiles: number
		droppedDirectory: boolean
		kept: { id: string; name: string; resolvedName: string; mediaCount: number }
	}
}

export type ConsoleSplitSeriesMutationVariables = Exact<{
	mediaIds: Array<string | number> | string | number
	name: string
}>

export type ConsoleSplitSeriesMutation = {
	splitSeries: {
		id: string
		name: string
		resolvedName: string
		mediaCount: number
		libraryId: string | null
	}
}

export type ConsoleKindleTargetsQueryVariables = Exact<{ [key: string]: never }>

export type ConsoleKindleTargetsQuery = {
	devices: Array<{ id: string; name: string; kindleEmail: string | null; revokedAt: string | null }>
}

export type ConsoleSendToKindleMutationVariables = Exact<{
	mediaId: string | number
	deviceId: string | number
}>

export type ConsoleSendToKindleMutation = {
	sendToKindle: {
		deviceId: string
		deviceName: string
		recipient: string
		format: string
		bytes: number
		converted: boolean
		note: string | null
	}
}

export type ConsoleKindleDeliveriesQueryVariables = Exact<{
	deviceId?: string | number | null | undefined
	mediaId?: string | number | null | undefined
	limit?: number | null | undefined
}>

export type ConsoleKindleDeliveriesQuery = {
	kindleDeliveries: Array<{
		id: string
		format: string
		bytes: number
		sentAt: string
		error: string | null
	}>
}

export type DeviceFieldsFragment = {
	id: string
	name: string
	kind: DeviceKind
	transformProfile: unknown
	libraryScope: Array<string> | null
	createdAt: string
	lastSeenAt: string | null
	lastSyncAt: string | null
	lastSyncSummary: unknown
	revokedAt: string | null
	telemetry: {
		batteryPercent: number | null
		charging: boolean | null
		batterySource: string | null
		batteryObservedAt: string | null
		syncStatus: string | null
		syncProtocol: DeviceProtocol | null
		syncedAt: string | null
		counters: {
			progress: number | null
			highlights: number | null
			notes: number | null
			bookmarks: number | null
			sessions: number | null
			items: number | null
		}
	} | null
	credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
}

export type DeviceCredentialFieldsFragment = {
	kind: DeviceCredentialKind
	protocol: DeviceProtocol
	credentialRef: string
	secret: string
}

export type DeviceEndpointFieldsFragment = {
	label: string
	url: string
	username: string | null
	secretHint: string
}

export type DevicesQueryVariables = Exact<{ [key: string]: never }>

export type DevicesQuery = {
	devices: Array<{
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}>
}

export type CreateDeviceMutationVariables = Exact<{
	kind: DeviceKind
	name?: string | null | undefined
}>

export type CreateDeviceMutation = {
	createDevice: {
		device: {
			id: string
			name: string
			kind: DeviceKind
			transformProfile: unknown
			libraryScope: Array<string> | null
			createdAt: string
			lastSeenAt: string | null
			lastSyncAt: string | null
			lastSyncSummary: unknown
			revokedAt: string | null
			telemetry: {
				batteryPercent: number | null
				charging: boolean | null
				batterySource: string | null
				batteryObservedAt: string | null
				syncStatus: string | null
				syncProtocol: DeviceProtocol | null
				syncedAt: string | null
				counters: {
					progress: number | null
					highlights: number | null
					notes: number | null
					bookmarks: number | null
					sessions: number | null
					items: number | null
				}
			} | null
			credential: {
				kind: DeviceCredentialKind
				protocol: DeviceProtocol
				secretHint: string
			} | null
		}
		credential: {
			kind: DeviceCredentialKind
			protocol: DeviceProtocol
			credentialRef: string
			secret: string
		}
		endpoints: Array<{ label: string; url: string; username: string | null; secretHint: string }>
	}
}

export type RenameDeviceMutationVariables = Exact<{
	id: string
	name: string
}>

export type RenameDeviceMutation = {
	renameDevice: {
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}
}

export type RotateDeviceCredentialMutationVariables = Exact<{
	id: string
}>

export type RotateDeviceCredentialMutation = {
	rotateDeviceCredential: {
		device: {
			id: string
			name: string
			kind: DeviceKind
			transformProfile: unknown
			libraryScope: Array<string> | null
			createdAt: string
			lastSeenAt: string | null
			lastSyncAt: string | null
			lastSyncSummary: unknown
			revokedAt: string | null
			telemetry: {
				batteryPercent: number | null
				charging: boolean | null
				batterySource: string | null
				batteryObservedAt: string | null
				syncStatus: string | null
				syncProtocol: DeviceProtocol | null
				syncedAt: string | null
				counters: {
					progress: number | null
					highlights: number | null
					notes: number | null
					bookmarks: number | null
					sessions: number | null
					items: number | null
				}
			} | null
			credential: {
				kind: DeviceCredentialKind
				protocol: DeviceProtocol
				secretHint: string
			} | null
		}
		credential: {
			kind: DeviceCredentialKind
			protocol: DeviceProtocol
			credentialRef: string
			secret: string
		}
		endpoints: Array<{ label: string; url: string; username: string | null; secretHint: string }>
	}
}

export type RevokeDeviceMutationVariables = Exact<{
	id: string
}>

export type RevokeDeviceMutation = {
	revokeDevice: {
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}
}

export type SetDeviceTransformProfileMutationVariables = Exact<{
	id: string
	profile?: unknown
}>

export type SetDeviceTransformProfileMutation = {
	setDeviceTransformProfile: {
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}
}

export type SetDeviceLibraryScopeMutationVariables = Exact<{
	id: string
	libraryIds?: Array<string | number> | string | number | null | undefined
}>

export type SetDeviceLibraryScopeMutation = {
	setDeviceLibraryScope: {
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}
}

export type SetDeviceKindleEmailMutationVariables = Exact<{
	id: string
	email?: string | null | undefined
}>

export type SetDeviceKindleEmailMutation = {
	setDeviceKindleEmail: {
		id: string
		name: string
		kind: DeviceKind
		transformProfile: unknown
		libraryScope: Array<string> | null
		createdAt: string
		lastSeenAt: string | null
		lastSyncAt: string | null
		lastSyncSummary: unknown
		revokedAt: string | null
		telemetry: {
			batteryPercent: number | null
			charging: boolean | null
			batterySource: string | null
			batteryObservedAt: string | null
			syncStatus: string | null
			syncProtocol: DeviceProtocol | null
			syncedAt: string | null
			counters: {
				progress: number | null
				highlights: number | null
				notes: number | null
				bookmarks: number | null
				sessions: number | null
				items: number | null
			}
		} | null
		credential: { kind: DeviceCredentialKind; protocol: DeviceProtocol; secretHint: string } | null
	}
}

export type PendingDevicePairingsQueryVariables = Exact<{ [key: string]: never }>

export type PendingDevicePairingsQuery = {
	pendingDevicePairings: Array<{
		id: string
		kind: DeviceKind
		name: string | null
		remoteIp: string
		status: DevicePairingStatus
		failedAttempts: number
		credentialIssued: boolean
		createdAt: string
		expiresAt: string
		approvedAt: string | null
	}>
}

export type ApproveDevicePairingMutationVariables = Exact<{
	pairingId: string | number
	code?: string | null | undefined
}>

export type ApproveDevicePairingMutation = {
	approveDevicePairing: { id: string; status: DevicePairingStatus }
}

export type DenyDevicePairingMutationVariables = Exact<{
	pairingId: string | number
}>

export type DenyDevicePairingMutation = {
	denyDevicePairing: { id: string; status: DevicePairingStatus }
}

export type DeviceSeenSubscriptionVariables = Exact<{
	deviceId?: string | null | undefined
}>

export type DeviceSeenSubscription = {
	deviceSeen: { deviceId: string; userId: string; protocol: DeviceProtocol }
}

export type ReadingStatsQueryVariables = Exact<{
	span: ReadingStatsSpan
	deviceId?: string | null | undefined
}>

export type ReadingStatsQuery = {
	readingStats: {
		from: string | null
		to: string
		sessions: number
		minutes: number
		pages: number
		booksFinished: number
		streakDays: number
		days: Array<{ date: string; sessions: number; minutes: number; pages: number }>
		devices: Array<{
			deviceId: string
			name: string | null
			kind: DeviceKind | null
			sessions: number
			minutes: number
			pages: number
		}>
	}
}

export type MyLoginActivityQueryVariables = Exact<{
	userId: string | number
}>

export type MyLoginActivityQuery = {
	loginActivityById: Array<{
		id: number
		ipAddress: string
		userAgent: string
		authenticationSuccessful: boolean
		timestamp: string
	}>
}

export type RuntimeComponentsQueryVariables = Exact<{ [key: string]: never }>

export type RuntimeComponentsQuery = {
	runtimeComponents: Array<{
		key: string
		label: string
		description: string
		category: string
		compiled: boolean
		desiredEnabled: boolean
		effectiveEnabled: boolean
		transitionMode: RuntimeTransitionMode
		transitionReason: string
		dependencies: Array<string>
		health: string
		restartRequired: boolean
		usageStatus: RuntimeUsageStatus
		usageEvidence: string | null
		activityCount: number | null
		lastActivityAt: string | null
		lastTransitionAt: string | null
		lastError: string | null
		ownedGauges: Array<{ name: string; kind: RuntimeGaugeKind; value: number }>
	}>
	runtimeMemory: {
		totalProcessRssBytes: number | null
		rssAvailable: boolean
		rssUnavailableReason: string | null
		anonymousPssBytes: number | null
		fileBackedPssBytes: number | null
		privateDirtyBytes: number | null
		memoryBreakdownAvailable: boolean
		memoryBreakdownUnavailableReason: string | null
	}
}

export type SetRuntimeComponentEnabledMutationVariables = Exact<{
	key: string
	enabled: boolean
}>

export type SetRuntimeComponentEnabledMutation = {
	setRuntimeComponentEnabled: {
		key: string
		label: string
		description: string
		category: string
		compiled: boolean
		desiredEnabled: boolean
		effectiveEnabled: boolean
		transitionMode: RuntimeTransitionMode
		transitionReason: string
		dependencies: Array<string>
		health: string
		restartRequired: boolean
		usageStatus: RuntimeUsageStatus
		usageEvidence: string | null
		activityCount: number | null
		lastActivityAt: string | null
		lastTransitionAt: string | null
		lastError: string | null
		ownedGauges: Array<{ name: string; kind: RuntimeGaugeKind; value: number }>
	}
}

export type DeviceCapabilitiesQueryVariables = Exact<{ [key: string]: never }>

export type DeviceCapabilitiesQuery = {
	deviceCapabilities: Array<{
		kind: DeviceKind
		protocol: DeviceProtocol
		componentKey: string
		compiled: boolean
		enabled: boolean
		available: boolean
		reason: string | null
	}>
}

export type ReaderLocatorFieldsFragment = {
	chapterTitle: string
	href: string
	title: string | null
	type: string
	locations: {
		fragments: Array<string> | null
		progression: unknown
		position: number | null
		totalProgression: unknown
		cssSelector: string | null
		partialCfi: string | null
	} | null
	text: { before: string | null; highlight: string | null; after: string | null } | null
}

export type ReaderBookQueryVariables = Exact<{
	id: string | number
}>

export type ReaderBookQuery = {
	mediaById: {
		id: string
		resolvedName: string
		extension: string
		pages: number
		seriesId: string | null
		series: { id: string; name: string }
		readProgress: {
			page: number | null
			positionMs: number | null
			percentageCompleted: unknown
			elapsedSeconds: number
			updatedAt: string | null
			locator: {
				chapterTitle: string
				href: string
				title: string | null
				type: string
				locations: {
					fragments: Array<string> | null
					progression: unknown
					position: number | null
					totalProgression: unknown
					cssSelector: string | null
					partialCfi: string | null
				} | null
				text: { before: string | null; highlight: string | null; after: string | null } | null
			} | null
		} | null
		audio: {
			durationMs: number
			codec: string
			chapterSource: AudioChapterSource
			tracks: Array<{
				index: number
				mime: string
				durationMs: number
				startOffsetMs: number
				byteSize: number
				url: string
			}>
			chapters: Array<{
				index: number
				title: string | null
				startMs: number
				endMs: number | null
			}>
		} | null
	} | null
}

export type ReaderVisiblePagesQueryVariables = Exact<{
	id: string | number
}>

export type ReaderVisiblePagesQuery = { mediaVisiblePages: Array<number> }

export type ReaderAnnotationsQueryVariables = Exact<{
	id: string | number
}>

export type ReaderAnnotationsQuery = {
	annotations: {
		items: Array<{
			id: string
			kind: AnnotationKind
			source: DeviceKind
			sourceDeviceName: string | null
			editable: boolean
			chapterTitle: string | null
			href: string | null
			fragment: string | null
			page: number | null
			progression: number | null
			excerpt: string | null
			note: string | null
			color: string | null
		}>
	}
	annotationsByMediaId: Array<{
		id: string
		annotationText: string | null
		locator: {
			chapterTitle: string
			href: string
			title: string | null
			type: string
			locations: {
				fragments: Array<string> | null
				progression: unknown
				position: number | null
				totalProgression: unknown
				cssSelector: string | null
				partialCfi: string | null
			} | null
			text: { before: string | null; highlight: string | null; after: string | null } | null
		}
	}>
}

export type ReaderUpdateProgressMutationVariables = Exact<{
	id: string | number
	input: MediaProgressInput
}>

export type ReaderUpdateProgressMutation = {
	updateMediaProgress: {
		id: number
		endPage: number | null
		endPercentage: unknown
		endLocator: {
			chapterTitle: string
			href: string
			title: string | null
			type: string
			locations: {
				fragments: Array<string> | null
				progression: unknown
				position: number | null
				totalProgression: unknown
				cssSelector: string | null
				partialCfi: string | null
			} | null
			text: { before: string | null; highlight: string | null; after: string | null } | null
		} | null
	}
}

export type ReaderEditionsQueryVariables = Exact<{
	id: string | number
}>

export type ReaderEditionsQuery = {
	mediaById: {
		audio: { durationMs: number } | null
		editions: Array<{
			id: string
			resolvedName: string
			extension: string
			audio: { durationMs: number } | null
			pairedPosition: {
				sourceMediaId: string
				positionMs: number | null
				progression: number
				confidence: number
				approximate: boolean
				locator: {
					chapterTitle: string
					href: string
					title: string | null
					type: string
					locations: {
						fragments: Array<string> | null
						progression: unknown
						position: number | null
						totalProgression: unknown
						cssSelector: string | null
						partialCfi: string | null
					} | null
					text: { before: string | null; highlight: string | null; after: string | null } | null
				} | null
			} | null
		}>
		editionSuggestions: Array<{
			workId: string
			status: PairStatus
			evidence: PairEvidence | null
			media: { id: string; resolvedName: string; extension: string }
		}>
	} | null
}

export type ReaderConfirmEditionPairMutationVariables = Exact<{
	mediaIdA: string | number
	mediaIdB: string | number
}>

export type ReaderConfirmEditionPairMutation = {
	confirmEditionPair: {
		workId: string | null
		status: PairStatus | null
		changed: boolean
		message: string | null
		chapterMapEntries: number
	}
}

export type ReaderRejectEditionPairMutationVariables = Exact<{
	mediaIdA: string | number
	mediaIdB: string | number
}>

export type ReaderRejectEditionPairMutation = {
	rejectEditionPair: {
		workId: string | null
		status: PairStatus | null
		changed: boolean
		message: string | null
	}
}

export type ReaderChapterMapMediaQueryVariables = Exact<{
	id: string | number
}>

export type ReaderChapterMapMediaQuery = {
	mediaById: {
		ebook: { spine: Array<{ idref: string; id: string | null; linear: boolean }> } | null
		audio: { chapters: Array<{ index: number; title: string | null }> } | null
	} | null
}

export type ReaderChapterMapQueryVariables = Exact<{
	ebookMediaId: string | number
	audioMediaId: string | number
}>

export type ReaderChapterMapQuery = {
	chapterMap: Array<{
		ebookMediaId: string
		audioMediaId: string
		ebookSpineIndex: number
		audioChapterIndex: number
		confidence: number
	}>
}

export type ReaderSetChapterMapEntryMutationVariables = Exact<{
	ebookMediaId: string | number
	audioMediaId: string | number
	ebookSpineIndex: number
	audioChapterIndex: number
	confidence?: number | null | undefined
}>

export type ReaderSetChapterMapEntryMutation = {
	setChapterMapEntry: {
		ebookMediaId: string
		audioMediaId: string
		ebookSpineIndex: number
		audioChapterIndex: number
		confidence: number
	}
}

export type ReaderClearChapterMapEntryMutationVariables = Exact<{
	ebookMediaId: string | number
	audioMediaId: string | number
	ebookSpineIndex: number
}>

export type ReaderClearChapterMapEntryMutation = { clearChapterMapEntry: boolean }

export type BookRequestFieldsFragment = {
	id: string
	requesterId: string
	title: string
	authors: string | null
	coverUrl: string | null
	internalMediaId: string | null
	internalWorkId: string | null
	sourceProvider: string | null
	remoteId: string | null
	externalKey: string | null
	destinationShelfId: string | null
	destinationDeviceId: string | null
	status: BookRequestStatus
	approvalPolicy: string
	automationEnabled: boolean
	scoringFloor: number
	verificationThreshold: number
	maxRetries: number
	retries: number
	approvedBy: string | null
	rejectedBy: string | null
	failureCode: string | null
	failureMessage: string | null
	createdAt: string
	updatedAt: string
	approvedAt: string | null
	completedAt: string | null
}

export type BookRequestReleaseFieldsFragment = {
	id: string
	searchId: string
	requestId: string
	sourceProvider: string
	remoteId: string
	externalKey: string | null
	title: string
	authors: string | null
	format: string | null
	language: string | null
	edition: string | null
	quality: string | null
	sizeBytes: number | null
	seeders: number | null
	previewName: string | null
	previewMime: string | null
	previewBytes: number | null
	score: number
	scoreComponents: unknown
	rank: number
	selected: boolean
}

export type BookRequestGrabFieldsFragment = {
	id: string
	requestId: string
	releaseId: string
	opaqueId: string
	status: string
	attempts: number
	maxAttempts: number
	failureCode: string | null
	failureMessage: string | null
	startedAt: string | null
	finishedAt: string | null
	lastPolledAt: string | null
	nextPollAt: string | null
	createdAt: string
	updatedAt: string
}

export type BookRequestGatewayFieldsFragment = {
	id: string
	endpoint: string
	enabled: boolean
	requireApproval: boolean
	automationEnabled: boolean
	scoringFloor: number
	verificationThreshold: number
	maxRetries: number
	handoffRoot: string | null
	hasToken: boolean
	tokenRedacted: string
	updatedBy: string | null
	updatedAt: string
}

export type BookRequestsQueryVariables = Exact<{
	status?: BookRequestStatus | null | undefined
	mineOnly?: boolean | null | undefined
	limit?: number | null | undefined
	offset?: number | null | undefined
}>

export type BookRequestsQuery = {
	bookRequests: Array<{
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}>
}

export type RequestDestinationsQueryVariables = Exact<{ [key: string]: never }>

export type RequestDestinationsQuery = {
	devices: Array<{ id: string; name: string; revokedAt: string | null }>
	readingLists: { nodes: Array<{ id: string; name: string }> }
}

export type BookRequestQueryVariables = Exact<{
	id: string | number
}>

export type BookRequestQuery = {
	bookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	} | null
}

export type BookRequestReleasesQueryVariables = Exact<{
	requestId: string | number
}>

export type BookRequestReleasesQuery = {
	bookRequestReleases: Array<{
		id: string
		searchId: string
		requestId: string
		sourceProvider: string
		remoteId: string
		externalKey: string | null
		title: string
		authors: string | null
		format: string | null
		language: string | null
		edition: string | null
		quality: string | null
		sizeBytes: number | null
		seeders: number | null
		previewName: string | null
		previewMime: string | null
		previewBytes: number | null
		score: number
		scoreComponents: unknown
		rank: number
		selected: boolean
	}>
}

export type BookRequestGatewayQueryVariables = Exact<{ [key: string]: never }>

export type BookRequestGatewayQuery = {
	bookRequestGateway: {
		id: string
		endpoint: string
		enabled: boolean
		requireApproval: boolean
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		handoffRoot: string | null
		hasToken: boolean
		tokenRedacted: string
		updatedBy: string | null
		updatedAt: string
	} | null
}

export type CreateBookRequestMutationVariables = Exact<{
	input: CreateBookRequestInput
}>

export type CreateBookRequestMutation = {
	createBookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type ApproveBookRequestMutationVariables = Exact<{
	requestId: string | number
	reason?: string | null | undefined
}>

export type ApproveBookRequestMutation = {
	approveBookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type RejectBookRequestMutationVariables = Exact<{
	requestId: string | number
	reason?: string | null | undefined
}>

export type RejectBookRequestMutation = {
	rejectBookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type SearchBookRequestMutationVariables = Exact<{
	requestId: string | number
}>

export type SearchBookRequestMutation = {
	searchBookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type SelectBookRequestReleaseMutationVariables = Exact<{
	requestId: string | number
	releaseId: string | number
}>

export type SelectBookRequestReleaseMutation = {
	selectBookRequestRelease: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type GrabBookRequestMutationVariables = Exact<{
	requestId: string | number
}>

export type GrabBookRequestMutation = {
	grabBookRequest: {
		id: string
		requestId: string
		releaseId: string
		opaqueId: string
		status: string
		attempts: number
		maxAttempts: number
		failureCode: string | null
		failureMessage: string | null
		startedAt: string | null
		finishedAt: string | null
		lastPolledAt: string | null
		nextPollAt: string | null
		createdAt: string
		updatedAt: string
	}
}

export type PollBookRequestGrabMutationVariables = Exact<{
	grabId: string | number
}>

export type PollBookRequestGrabMutation = {
	pollBookRequestGrab: {
		id: string
		requestId: string
		releaseId: string
		opaqueId: string
		status: string
		attempts: number
		maxAttempts: number
		failureCode: string | null
		failureMessage: string | null
		startedAt: string | null
		finishedAt: string | null
		lastPolledAt: string | null
		nextPollAt: string | null
		createdAt: string
		updatedAt: string
	}
}

export type RetryBookRequestMutationVariables = Exact<{
	requestId: string | number
}>

export type RetryBookRequestMutation = {
	retryBookRequest: {
		id: string
		requesterId: string
		title: string
		authors: string | null
		coverUrl: string | null
		internalMediaId: string | null
		internalWorkId: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		destinationShelfId: string | null
		destinationDeviceId: string | null
		status: BookRequestStatus
		approvalPolicy: string
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		retries: number
		approvedBy: string | null
		rejectedBy: string | null
		failureCode: string | null
		failureMessage: string | null
		createdAt: string
		updatedAt: string
		approvedAt: string | null
		completedAt: string | null
	}
}

export type UpdateBookRequestGatewayMutationVariables = Exact<{
	input: BookRequestGatewayInput
}>

export type UpdateBookRequestGatewayMutation = {
	updateBookRequestGateway: {
		id: string
		endpoint: string
		enabled: boolean
		requireApproval: boolean
		automationEnabled: boolean
		scoringFloor: number
		verificationThreshold: number
		maxRetries: number
		handoffRoot: string | null
		hasToken: boolean
		tokenRedacted: string
		updatedBy: string | null
		updatedAt: string
	}
}

export type IncomingSocialRecommendationsQueryVariables = Exact<{ [key: string]: never }>

export type IncomingSocialRecommendationsQuery = {
	socialRecommendations: Array<{
		id: string
		direction: RecommendationDirection
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		coverUrl: string | null
		message: string | null
		createdAt: string
		expiresAt: string | null
		acceptedAt: string | null
		handoffState: RecommendationHandoffState
		requestId: string | null
		destinationShelfId: string | null
	}>
}

export type OutgoingSocialRecommendationsQueryVariables = Exact<{ [key: string]: never }>

export type OutgoingSocialRecommendationsQuery = {
	socialRecommendations: Array<{
		id: string
		direction: RecommendationDirection
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		coverUrl: string | null
		message: string | null
		createdAt: string
		expiresAt: string | null
		acceptedAt: string | null
		handoffState: RecommendationHandoffState
		requestId: string | null
		destinationShelfId: string | null
	}>
}

export type AdaptiveRecommendationsQueryVariables = Exact<{
	limit?: number | null | undefined
}>

export type AdaptiveRecommendationsQuery = {
	adaptiveRecommendations: Array<{
		targetKey: string
		title: string
		authors: string
		coverUrl: string | null
		reasonCode: string
		score: number
	}>
}

export type SocialShareGrantsQueryVariables = Exact<{
	incoming?: boolean | null | undefined
}>

export type SocialShareGrantsQuery = {
	socialShareGrants: Array<{
		id: string
		targetKey: string
		title: string
		authors: string
		scopes: Array<ShareScope>
		state: ShareState
		expiresAt: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
	}>
}

export type SocialOverlaysQueryVariables = Exact<{
	targetKey?: string | null | undefined
}>

export type SocialOverlaysQuery = {
	socialOverlays: Array<{
		id: string
		targetKey: string
		kind: ShareOverlayKind
		excerpt: string | null
		body: string | null
		progression: number | null
		percentage: number | null
		color: string | null
		capturedAt: string
		hidden: boolean
	}>
}

export type SocialUserSearchQueryVariables = Exact<{
	query: string
}>

export type SocialUserSearchQuery = { socialUserSearch: Array<{ id: string; username: string }> }

export type SocialPreferencesQueryVariables = Exact<{ [key: string]: never }>

export type SocialPreferencesQuery = {
	socialPreferences: { recommendationsOptOut: boolean; sharingOptOut: boolean; updatedAt: string }
}

export type SendRecommendationMutationVariables = Exact<{
	input: SendRecommendationInput
}>

export type SendRecommendationMutation = {
	sendRecommendation: {
		id: string
		direction: RecommendationDirection
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		coverUrl: string | null
		message: string | null
		createdAt: string
		expiresAt: string | null
		handoffState: RecommendationHandoffState
		requestId: string | null
		destinationShelfId: string | null
	}
}

export type RespondToRecommendationMutationVariables = Exact<{
	id: string | number
	response: RecommendationResponseInput
}>

export type RespondToRecommendationMutation = {
	respondToRecommendation: {
		id: string
		direction: RecommendationDirection
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		coverUrl: string | null
		message: string | null
		createdAt: string
		expiresAt: string | null
		acceptedAt: string | null
		handoffState: RecommendationHandoffState
		requestId: string | null
		destinationShelfId: string | null
	}
}

export type RevokeRecommendationMutationVariables = Exact<{
	id: string | number
}>

export type RevokeRecommendationMutation = {
	revokeRecommendation: {
		direction: RecommendationDirection
		id: string
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		createdAt: string
	}
}

export type DismissRecommendationMutationVariables = Exact<{
	id: string | number
}>

export type DismissRecommendationMutation = {
	dismissRecommendation: {
		direction: RecommendationDirection
		id: string
		state: RecommendationState
		title: string
	}
}

export type RequestRecommendationMutationVariables = Exact<{
	id: string | number
	destination?: RequestDestinationInput | null | undefined
}>

export type RequestRecommendationMutation = {
	requestRecommendation: {
		id: string
		direction: RecommendationDirection
		state: RecommendationState
		targetKind: RecommendationTargetKind
		title: string
		authors: string
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
		coverUrl: string | null
		message: string | null
		createdAt: string
		expiresAt: string | null
		acceptedAt: string | null
		handoffState: RecommendationHandoffState
		requestId: string | null
		destinationShelfId: string | null
	}
}

export type SetRecommendationOptOutMutationVariables = Exact<{
	optOut: boolean
}>

export type SetRecommendationOptOutMutation = {
	setRecommendationOptOut: {
		recommendationsOptOut: boolean
		sharingOptOut: boolean
		updatedAt: string
	}
}

export type CreateShareGrantMutationVariables = Exact<{
	input: CreateShareGrantInput
}>

export type CreateShareGrantMutation = {
	createShareGrant: {
		id: string
		targetKey: string
		title: string
		authors: string
		scopes: Array<ShareScope>
		state: ShareState
		expiresAt: string | null
		sourceProvider: string | null
		remoteId: string | null
		externalKey: string | null
	}
}

export type RespondToShareGrantMutationVariables = Exact<{
	id: string | number
	accept: boolean
}>

export type RespondToShareGrantMutation = {
	respondToShareGrant: {
		id: string
		targetKey: string
		title: string
		authors: string
		scopes: Array<ShareScope>
		state: ShareState
		expiresAt: string | null
	}
}

export type RevokeShareGrantMutationVariables = Exact<{
	id: string | number
}>

export type RevokeShareGrantMutation = {
	revokeShareGrant: { id: string; targetKey: string; state: ShareState }
}

export type SetOverlayVisibilityMutationVariables = Exact<{
	overlayId: string | number
	hidden: boolean
}>

export type SetOverlayVisibilityMutation = {
	setOverlayVisibility: {
		id: string
		targetKey: string
		kind: ShareOverlayKind
		excerpt: string | null
		body: string | null
		progression: number | null
		percentage: number | null
		color: string | null
		capturedAt: string
		hidden: boolean
	}
}

export type SocialRequestDestinationsQueryVariables = Exact<{ [key: string]: never }>

export type SocialRequestDestinationsQuery = {
	devices: Array<{ id: string; name: string; revokedAt: string | null }>
	readingLists: { nodes: Array<{ id: string; name: string }> }
}

export type SocialBookClubsQueryVariables = Exact<{
	all?: boolean | null | undefined
}>

export type SocialBookClubsQuery = {
	bookClubs: Array<{
		id: string
		name: string
		slug: string
		description: string | null
		isPrivate: boolean
		createdAt: string
		emoji: string | null
		membersCount: number
		membership: {
			id: string
			userId: string
			username: string
			displayName: string | null
			avatarUrl: string | null
			role: BookClubMemberRole
			hideProgress: boolean
			joinedAt: string
			isCreator: boolean
		} | null
		members: Array<{
			id: string
			userId: string
			username: string
			displayName: string | null
			avatarUrl: string | null
			role: BookClubMemberRole
			hideProgress: boolean
			joinedAt: string
			isCreator: boolean
		}>
		invitations: Array<{
			id: string
			role: BookClubMemberRole
			userId: string
			bookClubId: string
			user: { id: string; username: string }
		}>
	}>
}

export type MyBookClubInvitationsQueryVariables = Exact<{ [key: string]: never }>

export type MyBookClubInvitationsQuery = {
	myBookClubInvitations: Array<{
		id: string
		role: BookClubMemberRole
		userId: string
		bookClubId: string
		user: { id: string; username: string }
		bookClub: { id: string; name: string; slug: string }
	}>
}

export type CreateBookClubInvitationMutationVariables = Exact<{
	id: string | number
	input: BookClubInvitationInput
}>

export type CreateBookClubInvitationMutation = {
	createBookClubInvitation: {
		id: string
		role: BookClubMemberRole
		userId: string
		bookClubId: string
		user: { id: string; username: string }
	}
}

export type RespondToBookClubInvitationMutationVariables = Exact<{
	id: string | number
	input: BookClubInvitationResponseInput
}>

export type RespondToBookClubInvitationMutation = {
	respondToBookClubInvitation: {
		id: string
		role: BookClubMemberRole
		userId: string
		bookClubId: string
	}
}

export type RemoveBookClubMemberMutationVariables = Exact<{
	bookClubId: string | number
	memberId: string | number
}>

export type RemoveBookClubMemberMutation = {
	removeBookClubMember: {
		id: string
		userId: string
		username: string
		displayName: string | null
		avatarUrl: string | null
		role: BookClubMemberRole
		hideProgress: boolean
		joinedAt: string
		isCreator: boolean
	}
}

export type LeaveBookClubMutationVariables = Exact<{
	bookClubId: string | number
}>

export type LeaveBookClubMutation = {
	leaveBookClub: { id: string; userId: string; bookClubId: string; role: BookClubMemberRole }
}

export type CreateBookClubMutationVariables = Exact<{
	input: CreateBookClubInput
}>

export type CreateBookClubMutation = {
	createBookClub: {
		id: string
		name: string
		slug: string
		description: string | null
		isPrivate: boolean
		emoji: string | null
		membersCount: number
	}
}

export type WorkerFieldsFragment = {
	id: string
	name: string
	version: string | null
	kinds: Array<string>
	connectedAt: string
	lastSeenAt: string
}

export type WorkerJobFieldsFragment = {
	id: string
	kind: string
	status: WorkerJobStatus
	workerId: string | null
	priority: number
	progress: number
	message: string | null
	error: string | null
	createdAt: string
	startedAt: string | null
	finishedAt: string | null
}

export type WorkersQueryVariables = Exact<{ [key: string]: never }>

export type WorkersQuery = {
	workers: Array<{
		id: string
		name: string
		version: string | null
		kinds: Array<string>
		connectedAt: string
		lastSeenAt: string
	}>
	workerJobs: Array<{
		id: string
		kind: string
		status: WorkerJobStatus
		workerId: string | null
		priority: number
		progress: number
		message: string | null
		error: string | null
		createdAt: string
		startedAt: string | null
		finishedAt: string | null
	}>
}

export type CancelWorkerJobMutationVariables = Exact<{
	id: string
}>

export type CancelWorkerJobMutation = {
	cancelWorkerJob: {
		id: string
		kind: string
		status: WorkerJobStatus
		workerId: string | null
		priority: number
		progress: number
		message: string | null
		error: string | null
		createdAt: string
		startedAt: string | null
		finishedAt: string | null
	}
}

export type WorkerJobEventsSubscriptionVariables = Exact<{ [key: string]: never }>

export type WorkerJobEventsSubscription = {
	readEvents:
		| { __typename: 'AnalysisJobFailed' }
		| { __typename: 'CollectionAdded' }
		| { __typename: 'CollectionChanged' }
		| { __typename: 'CollectionDeleted' }
		| { __typename: 'CreatedManySeries' }
		| { __typename: 'CreatedMedia' }
		| { __typename: 'CreatedOrUpdatedManyMedia' }
		| { __typename: 'DevicePaired' }
		| { __typename: 'DevicePairingRequested' }
		| { __typename: 'DeviceSeen' }
		| { __typename: 'DiscoveredMissingLibrary' }
		| { __typename: 'IngestAwaitingReview' }
		| { __typename: 'IngestItemChanged' }
		| { __typename: 'JobOutput' }
		| { __typename: 'JobQueueStatus' }
		| { __typename: 'JobStarted' }
		| { __typename: 'JobUpdate' }
		| { __typename: 'LibraryCreated' }
		| { __typename: 'LibraryDeleted' }
		| { __typename: 'LibraryUpdated' }
		| { __typename: 'MediaDeleted' }
		| { __typename: 'ProviderCatalogRefreshed' }
		| { __typename: 'ProviderMatchDone' }
		| { __typename: 'ProviderSeriesMaterialized' }
		| { __typename: 'ProviderSourceHealthChanged' }
		| { __typename: 'QualityFailed' }
		| { __typename: 'ReadListAdded' }
		| { __typename: 'ReadListChanged' }
		| { __typename: 'ReadListDeleted' }
		| { __typename: 'SeriesDeleted' }
		| {
				__typename: 'WorkerJobChanged'
				id: string
				kind: string
				status: WorkerJobStatus
				workerId: string | null
				progress: number
				message: string | null
				error: string | null
		  }
}

export const ConsoleAnnotationFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleAnnotationFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'AnnotationEntry' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editable' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'fragment' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'book' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAnnotationFieldsFragment, unknown>
export const BookReadingLogDeviceFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookReadingLogDeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadingDevice' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'revoked' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookReadingLogDeviceFieldsFragment, unknown>
export const BookDetailMetadataFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailMetadataFieldsFragment, unknown>
export const BookDetailFileFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailFileFieldsFragment, unknown>
export const BookDetailAudioFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailAudioFieldsFragment, unknown>
export const BookDetailEditionFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailEditionFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookEdition' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailMetadataFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'file' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFileFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailAudioFields' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'pairEvidence' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailEditionFieldsFragment, unknown>
export const BookDetailReviewFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailReviewFieldsFragment, unknown>
export const BookDetailReadAloudFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadAloud' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'granularity' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generator' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generatorVersion' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'algorithm' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cueCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jobId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'map' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'artifact' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mimeType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cacheKey' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailReadAloudFieldsFragment, unknown>
export const BookDetailFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookDetail' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'workMetadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'author' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'metadata' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'editions' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailEditionFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mismatches' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'field' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audiobookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolved' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'review' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailReviewFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readAloud' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailEditionFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookEdition' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailMetadataFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'file' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFileFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailAudioFields' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'pairEvidence' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadAloud' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'granularity' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generator' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generatorVersion' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'algorithm' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cueCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jobId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'map' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'artifact' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mimeType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cacheKey' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailFieldsFragment, unknown>
export const ConnectionKindleDestinationFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'KindleDestination' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'email' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isDefault' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionKindleDestinationFieldsFragment, unknown>
export const ConnectionKindleDestinationDeliveryFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveryFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'KindleDestinationDelivery' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationEmail' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'recipient' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'converted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sentAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionKindleDestinationDeliveryFieldsFragment, unknown>
export const ConnectionHardcoverFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverConnection' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'connected' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUserId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUsername' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'capabilities' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'useForMetadata' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'importJournals' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'syncProgress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionHardcoverFieldsFragment, unknown>
export const ConnectionHardcoverLinkFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverLinkFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverMediaLink' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'linkedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionHardcoverLinkFieldsFragment, unknown>
export const ConnectionSmtpFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionSmtpFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'SmtpSettings' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'configured' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'senderEmail' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'senderDisplayName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'smtpHost' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'smtpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'tlsEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastUsedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionSmtpFieldsFragment, unknown>
export const CrosspointTargetFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointTargetFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointTarget' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hostOrIp' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'httpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'wsPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rootPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'discoveryMethod' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'fingerprint' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'serial' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'profile' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'optimizerEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetModel' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jpegQuality' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'grayscale' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'autoCrop' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'splitLargeParagraphs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'removeFonts' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'chunkBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryDelaySeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'timeoutSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'maxUploadBytes' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CrosspointTargetFieldsFragment, unknown>
export const CrosspointTargetVerificationFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointTargetVerificationFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'CrosspointTargetVerification' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'hostOrIp' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'httpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'wsPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'serial' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verified' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CrosspointTargetVerificationFieldsFragment, unknown>
export const CrosspointDeliveryFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointDelivery' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceRevision' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'idempotencyKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextAttemptAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'queuedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CrosspointDeliveryFieldsFragment, unknown>
export const DashboardBookCardFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DashboardBookCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Media' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readProgress' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'elapsedSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DashboardBookCardFragment, unknown>
export const ConsolePageInfoFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsolePageInfoFragment, unknown>
export const ConsoleLibraryCardFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleLibraryCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Library' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastScannedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'config' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryPattern' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'watch' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'stats' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'inProgressBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalBytes' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibraryCardFragment, unknown>
export const ConsoleSeriesCardFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleSeriesCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Series' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'readCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'unreadCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isComplete' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleSeriesCardFragment, unknown>
export const ConsoleBookRowFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleBookRow' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Media' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readProgress' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readHistory' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'readthroughNumber' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'dnf' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleBookRowFragment, unknown>
export const DeviceFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DeviceFieldsFragment, unknown>
export const DeviceCredentialFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceCredentialFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'IssuedDeviceCredential' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'credentialRef' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secret' } },
				],
			},
		},
	],
} as unknown as DocumentNode<DeviceCredentialFieldsFragment, unknown>
export const DeviceEndpointFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceEndpointFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'DeviceEndpoint' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
				],
			},
		},
	],
} as unknown as DocumentNode<DeviceEndpointFieldsFragment, unknown>
export const ReaderLocatorFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderLocatorFieldsFragment, unknown>
export const BookRequestFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestFieldsFragment, unknown>
export const BookRequestReleaseFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestReleaseFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestRelease' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'searchId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'edition' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'quality' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sizeBytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seeders' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewMime' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewBytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'score' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoreComponents' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rank' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'selected' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestReleaseFieldsFragment, unknown>
export const BookRequestGrabFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGrabFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestGrab' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'releaseId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'opaqueId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastPolledAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextPollAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestGrabFieldsFragment, unknown>
export const BookRequestGatewayFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGatewayFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'BookRequestGatewaySettings' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'endpoint' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requireApproval' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'handoffRoot' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hasToken' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'tokenRedacted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestGatewayFieldsFragment, unknown>
export const WorkerFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'WorkerFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Worker' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'version' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kinds' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<WorkerFieldsFragment, unknown>
export const WorkerJobFieldsFragmentDoc = {
	kind: 'Document',
	definitions: [
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'WorkerJobFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'WorkerJob' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workerId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'priority' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<WorkerJobFieldsFragment, unknown>
export const ConsoleAnnotationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleAnnotations' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'AnnotationFilterInput' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'OffsetPagination' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'total' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'hasNext' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'items' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'ConsoleAnnotationFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleAnnotationFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'AnnotationEntry' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editable' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'fragment' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'book' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAnnotationsQuery, ConsoleAnnotationsQueryVariables>
export const ConsoleAnnotationBooksDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleAnnotationBooks' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'AnnotationFilterInput' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'page' },
											value: { kind: 'IntValue', value: '1' },
										},
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'pageSize' },
											value: { kind: 'IntValue', value: '500' },
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'items' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'book' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAnnotationBooksQuery, ConsoleAnnotationBooksQueryVariables>
export const ConsoleUpdateAnnotationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleUpdateAnnotation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'UpdateAnnotationInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateAnnotation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'annotationText' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleUpdateAnnotationMutation,
	ConsoleUpdateAnnotationMutationVariables
>
export const ConsoleDeleteAnnotationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleDeleteAnnotation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deleteAnnotation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'id' } }],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleDeleteAnnotationMutation,
	ConsoleDeleteAnnotationMutationVariables
>
export const ConsoleAnnotationSinksDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleAnnotationSinks' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotationSinks' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'settings' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'valueType' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'required' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'secret' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'defaultValue' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'helpUrl' } },
										],
									},
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotationSyncStatus' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pending' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'sinks' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'sinkId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastRunAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAnnotationSinksQuery, ConsoleAnnotationSinksQueryVariables>
export const ConsoleSetAnnotationSinkSettingsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleSetAnnotationSinkSettings' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'sinkId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'settings' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'JSON' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setAnnotationSinkSettings' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'sinkId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'sinkId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'settings' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'settings' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'enabled' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pending' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'sinks' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'sinkId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastRunAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleSetAnnotationSinkSettingsMutation,
	ConsoleSetAnnotationSinkSettingsMutationVariables
>
export const ConsoleRunAnnotationSyncDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleRunAnnotationSync' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'runAnnotationSync' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pending' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'sinks' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'sinkId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastRunAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleRunAnnotationSyncMutation,
	ConsoleRunAnnotationSyncMutationVariables
>
export const BookDetailDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookDetail' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookDetail' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'workMetadata' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'author' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'metadata' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'editions' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'BookDetailEditionFields' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'mismatches' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'field' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'ebookValue' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'audiobookValue' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'workValue' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'resolved' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'review' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'BookDetailReviewFields' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'readAloud' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailEditionFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookEdition' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailMetadataFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'file' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFileFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailAudioFields' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'pairEvidence' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadAloud' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'granularity' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generator' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generatorVersion' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'algorithm' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cueCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jobId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'map' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'artifact' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mimeType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cacheKey' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookDetailQuery, BookDetailQueryVariables>
export const BookCoverDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookCover' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mediaById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'thumbnail' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookCoverQuery, BookCoverQueryVariables>
export const BookSearchDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookSearch' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'query' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'searchBooks' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'query' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'query' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'score' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookSearchQuery, BookSearchQueryVariables>
export const BookReadingLogDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookReadingLog' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookReadingLog' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'editions' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'head' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'locator' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'ReaderLocatorFields' },
																	},
																],
															},
														},
														{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'trackIndex' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'completed' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'sourceProtocol' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceId' } },
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'sourceDevice' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'BookReadingLogDeviceFields' },
																	},
																],
															},
														},
													],
												},
											},
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'sessions' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'sessionDate' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'readthroughNumber' } },
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'startLocator' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'ReaderLocatorFields' },
																	},
																],
															},
														},
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'endLocator' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'ReaderLocatorFields' },
																	},
																],
															},
														},
														{ kind: 'Field', name: { kind: 'Name', value: 'startPage' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'endPage' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'endPositionMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'startPercentage' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'endPercentage' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'elapsedSeconds' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'sourceProtocol' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceIds' } },
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'sourceDevices' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'BookReadingLogDeviceFields' },
																	},
																],
															},
														},
														{ kind: 'Field', name: { kind: 'Name', value: 'liseurSessionId' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookReadingLogDeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadingDevice' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'revoked' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookReadingLogQuery, BookReadingLogQueryVariables>
export const BookHighlightsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookHighlights' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'mediaId' },
											value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
										},
									],
								},
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'page' },
											value: { kind: 'IntValue', value: '1' },
										},
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'pageSize' },
											value: { kind: 'IntValue', value: '1000' },
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'items' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'editable' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'fragment' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'book' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookHighlightsQuery, BookHighlightsQueryVariables>
export const SimilarBooksDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SimilarBooks' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'similarBooks' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'score' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SimilarBooksQuery, SimilarBooksQueryVariables>
export const BookMetadataCandidatesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookMetadataCandidates' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookMetadataCandidates' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'rawHits' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'addedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'matchCandidates' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'provider' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'externalId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'metadata' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{
															kind: 'InlineFragment',
															typeCondition: {
																kind: 'NamedType',
																name: { kind: 'Name', value: 'ExternalMediaMetadata' },
															},
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{ kind: 'Field', name: { kind: 'Name', value: 'provider' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'externalId' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'seriesName' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'tags' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'isbn' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'isbn13' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'artists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'providerUrl' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'subtitle' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
																	{
																		kind: 'Field',
																		name: { kind: 'Name', value: 'runtimeMinutes' },
																	},
																],
															},
														},
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<BookMetadataCandidatesQuery, BookMetadataCandidatesQueryVariables>
export const SearchBookMetadataDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SearchBookMetadata' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'search' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadataSearchInput' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'searchBookMetadata' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'search' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'search' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'rawHits' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'addedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'matchCandidates' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'provider' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'externalId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'metadata' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{
															kind: 'InlineFragment',
															typeCondition: {
																kind: 'NamedType',
																name: { kind: 'Name', value: 'ExternalMediaMetadata' },
															},
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{ kind: 'Field', name: { kind: 'Name', value: 'provider' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'externalId' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'seriesName' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'tags' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'isbn' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'isbn13' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'artists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'providerUrl' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'subtitle' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
																	{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
																	{
																		kind: 'Field',
																		name: { kind: 'Name', value: 'runtimeMinutes' },
																	},
																],
															},
														},
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SearchBookMetadataQuery, SearchBookMetadataQueryVariables>
export const ApplyBookMetadataDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ApplyBookMetadata' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookMetadataApplyInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'applyBookMetadata' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailEditionFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookEdition' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailMetadataFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'file' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFileFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailAudioFields' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'pairEvidence' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadAloud' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'granularity' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generator' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generatorVersion' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'algorithm' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cueCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jobId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'map' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'artifact' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mimeType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cacheKey' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookDetail' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'workMetadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'author' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'metadata' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'editions' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailEditionFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mismatches' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'field' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audiobookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolved' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'review' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailReviewFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readAloud' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ApplyBookMetadataMutation, ApplyBookMetadataMutationVariables>
export const ApplyBookMetadataCandidateDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ApplyBookMetadataCandidate' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'candidateIndex' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'scope' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookMetadataScope' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'selectedFields' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'MetadataField' } },
							},
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'applyBookMetadataCandidate' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'candidateIndex' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'candidateIndex' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'scope' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'scope' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'selectedFields' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'selectedFields' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailMetadataFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaMetadata' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'titleSort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesGroup' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArc' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'storyArcNumber' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'number' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'month' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'day' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pencillers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'inkers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'colorists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'letterers' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverArtists' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'editors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'narrators' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'teams' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pageCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierAmazon' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierCalibre' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierGoogle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierIsbn' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierMobiAsin' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'identifierUuid' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataSource' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'metadataExternalId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFileFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookFile' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'koreaderHash' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'modifiedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailAudioFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookAudioFacts' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sampleRate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'channels' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bitrate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapters' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailEditionFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookEdition' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailMetadataFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'file' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailFileFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailAudioFields' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'pairEvidence' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReadAloud' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncMap' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'granularity' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generator' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'generatorVersion' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'algorithm' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cueCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jobId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'map' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'artifact' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mimeType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cacheKey' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookDetail' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'workMetadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'author' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'metadata' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lockedFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'editions' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailEditionFields' },
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mismatches' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'field' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audiobookValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'workValue' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolved' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'review' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailReviewFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readAloud' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookDetailReadAloudFields' },
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ApplyBookMetadataCandidateMutation,
	ApplyBookMetadataCandidateMutationVariables
>
export const UpsertBookReviewDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'UpsertBookReview' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReviewInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'upsertBookReview' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookDetailReviewFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookDetailReviewFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookReview' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rating' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'content' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<UpsertBookReviewMutation, UpsertBookReviewMutationVariables>
export const DeleteBookReviewDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'DeleteBookReview' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deleteBookReview' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DeleteBookReviewMutation, DeleteBookReviewMutationVariables>
export const ConnectionKindleDestinationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionKindleDestinations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'kindleDestinations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionKindleDestinationFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'KindleDestination' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'email' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isDefault' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionKindleDestinationsQuery,
	ConnectionKindleDestinationsQueryVariables
>
export const ConnectionUpsertKindleDestinationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionUpsertKindleDestination' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'email' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'makeDefault' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'upsertKindleDestination' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'name' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'email' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'email' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'makeDefault' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'makeDefault' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionKindleDestinationFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'KindleDestination' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'email' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isDefault' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionUpsertKindleDestinationMutation,
	ConnectionUpsertKindleDestinationMutationVariables
>
export const ConnectionDeleteKindleDestinationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionDeleteKindleDestination' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deleteKindleDestination' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionDeleteKindleDestinationMutation,
	ConnectionDeleteKindleDestinationMutationVariables
>
export const ConnectionKindleDestinationDeliveriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'kindleDestinationDeliveries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveryFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'KindleDestinationDelivery' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationEmail' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'recipient' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'converted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sentAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionKindleDestinationDeliveriesQuery,
	ConnectionKindleDestinationDeliveriesQueryVariables
>
export const ConnectionSendToKindleDestinationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionSendToKindleDestination' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'destinationId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'sendToKindleDestination' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'destinationId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'destinationId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionKindleDestinationDeliveryFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'KindleDestinationDelivery' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationEmail' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'recipient' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'bytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'converted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sentAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionSendToKindleDestinationMutation,
	ConnectionSendToKindleDestinationMutationVariables
>
export const ConnectionHardcoverDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionHardcover' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'hardcoverConnection' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverConnection' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'connected' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUserId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUsername' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'capabilities' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'useForMetadata' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'importJournals' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'syncProgress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionHardcoverQuery, ConnectionHardcoverQueryVariables>
export const ConnectionConnectHardcoverDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionConnectHardcover' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'apiToken' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'useForMetadata' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: true },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'importJournals' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: false },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'syncProgress' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: false },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'connectHardcover' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'apiToken' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'apiToken' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'useForMetadata' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'useForMetadata' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'importJournals' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'importJournals' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'syncProgress' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'syncProgress' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverConnection' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'connected' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUserId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUsername' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'capabilities' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'useForMetadata' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'importJournals' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'syncProgress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionConnectHardcoverMutation,
	ConnectionConnectHardcoverMutationVariables
>
export const ConnectionUpdateHardcoverDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionUpdateHardcover' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'useForMetadata' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'importJournals' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'syncProgress' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateHardcoverConnection' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'useForMetadata' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'useForMetadata' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'importJournals' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'importJournals' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'syncProgress' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'syncProgress' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverConnection' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'connected' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUserId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteUsername' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'capabilities' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'useForMetadata' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'importJournals' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'syncProgress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionUpdateHardcoverMutation,
	ConnectionUpdateHardcoverMutationVariables
>
export const ConnectionDisconnectHardcoverDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionDisconnectHardcover' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [{ kind: 'Field', name: { kind: 'Name', value: 'disconnectHardcover' } }],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionDisconnectHardcoverMutation,
	ConnectionDisconnectHardcoverMutationVariables
>
export const ConnectionAdminEmailersDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionAdminEmailers' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'emailers' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isPrimary' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderEmail' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderDisplayName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpHost' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpPort' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'tlsEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastUsedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionAdminEmailersQuery, ConnectionAdminEmailersQueryVariables>
export const ConnectionCreateAdminEmailerDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionCreateAdminEmailer' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'EmailerInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createEmailer' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isPrimary' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderEmail' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderDisplayName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpHost' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpPort' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'tlsEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastUsedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionCreateAdminEmailerMutation,
	ConnectionCreateAdminEmailerMutationVariables
>
export const ConnectionUpdateAdminEmailerDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionUpdateAdminEmailer' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'EmailerInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateEmailer' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isPrimary' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderEmail' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'senderDisplayName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpHost' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'smtpPort' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'tlsEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastUsedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionUpdateAdminEmailerMutation,
	ConnectionUpdateAdminEmailerMutationVariables
>
export const ConnectionTestAdminEmailerDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionTestAdminEmailer' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'config' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'EmailerClientConfig' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'recipient' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'testEmailer' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'config' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'config' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'recipient' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'recipient' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionTestAdminEmailerMutation,
	ConnectionTestAdminEmailerMutationVariables
>
export const ConnectionHardcoverLinksDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionHardcoverLinks' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'hardcoverMediaLinks' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionHardcoverLinkFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverLinkFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverMediaLink' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'linkedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionHardcoverLinksQuery, ConnectionHardcoverLinksQueryVariables>
export const ConnectionLinkHardcoverMediaDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionLinkHardcoverMedia' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'remoteId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'linkHardcoverMedia' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'remoteId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'remoteId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'ConnectionHardcoverLinkFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionHardcoverLinkFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'HardcoverMediaLink' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'linkedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionLinkHardcoverMediaMutation,
	ConnectionLinkHardcoverMediaMutationVariables
>
export const ConnectionUnlinkHardcoverMediaDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionUnlinkHardcoverMedia' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'unlinkHardcoverMedia' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionUnlinkHardcoverMediaMutation,
	ConnectionUnlinkHardcoverMediaMutationVariables
>
export const ConnectionSmtpDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionSmtp' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'smtpSettings' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConnectionSmtpFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConnectionSmtpFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'SmtpSettings' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'configured' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'senderEmail' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'senderDisplayName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'smtpHost' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'smtpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'tlsEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastUsedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ConnectionSmtpQuery, ConnectionSmtpQueryVariables>
export const ConnectionAdminBoundaryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConnectionAdminBoundary' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [{ kind: 'Field', name: { kind: 'Name', value: 'annotationSyncRoot' } }],
			},
		},
	],
} as unknown as DocumentNode<ConnectionAdminBoundaryQuery, ConnectionAdminBoundaryQueryVariables>
export const ConnectionSyncHardcoverNowDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConnectionSyncHardcoverNow' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'syncHardcoverNow' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'imported' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'unresolved' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'projected' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'skipped' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConnectionSyncHardcoverNowMutation,
	ConnectionSyncHardcoverNowMutationVariables
>
export const CrosspointTargetDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'CrosspointTarget' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'crosspointTarget' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'CrosspointTargetFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointTargetFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointTarget' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hostOrIp' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'httpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'wsPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rootPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'discoveryMethod' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'fingerprint' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'serial' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'profile' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'optimizerEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetModel' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jpegQuality' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'grayscale' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'autoCrop' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'splitLargeParagraphs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'removeFonts' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'chunkBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryDelaySeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'timeoutSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'maxUploadBytes' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CrosspointTargetQuery, CrosspointTargetQueryVariables>
export const VerifyCrosspointTargetDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'VerifyCrosspointTarget' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'host' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'verifyCrosspointTarget' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'host' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'host' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'CrosspointTargetVerificationFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointTargetVerificationFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'CrosspointTargetVerification' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'hostOrIp' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'httpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'wsPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'serial' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verified' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	VerifyCrosspointTargetMutation,
	VerifyCrosspointTargetMutationVariables
>
export const UpdateCrosspointTargetDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'UpdateCrosspointTarget' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointTargetInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateCrosspointTarget' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'CrosspointTargetFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointTargetFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointTarget' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hostOrIp' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'httpPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'wsPort' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rootPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'discoveryMethod' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verifiedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'fingerprint' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'model' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'serial' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'profile' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'optimizerEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetModel' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'jpegQuality' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'grayscale' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'autoCrop' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'splitLargeParagraphs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'removeFonts' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'chunkBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'retryDelaySeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'timeoutSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'maxUploadBytes' } },
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	UpdateCrosspointTargetMutation,
	UpdateCrosspointTargetMutationVariables
>
export const CrosspointDeliveriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'CrosspointDeliveries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'crosspointDeliveries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointDelivery' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceRevision' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'idempotencyKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextAttemptAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'queuedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CrosspointDeliveriesQuery, CrosspointDeliveriesQueryVariables>
export const QueueCrosspointDeliveriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'QueueCrosspointDeliveries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
							},
						},
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'targetPath' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'queueCrosspointDeliveries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIds' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'targetPath' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'targetPath' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointDelivery' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceRevision' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'idempotencyKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextAttemptAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'queuedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	QueueCrosspointDeliveriesMutation,
	QueueCrosspointDeliveriesMutationVariables
>
export const RetryCrosspointDeliveryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RetryCrosspointDelivery' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'retryCrosspointDelivery' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointDelivery' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceRevision' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'idempotencyKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextAttemptAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'queuedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	RetryCrosspointDeliveryMutation,
	RetryCrosspointDeliveryMutationVariables
>
export const CancelCrosspointDeliveryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CancelCrosspointDelivery' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'cancelCrosspointDelivery' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'CrosspointDeliveryFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'CrosspointDelivery' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceRevision' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileDigest' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'profileJson' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationPath' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'idempotencyKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextAttemptAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'queuedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	CancelCrosspointDeliveryMutation,
	CancelCrosspointDeliveryMutationVariables
>
export const DashboardViewerDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DashboardViewer' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'me' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isServerOwner' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'permissions' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DashboardViewerQuery, DashboardViewerQueryVariables>
export const DashboardKeepReadingDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DashboardKeepReading' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'keepReading' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DashboardBookCard' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DashboardBookCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Media' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readProgress' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'elapsedSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DashboardKeepReadingQuery, DashboardKeepReadingQueryVariables>
export const DashboardRecentlyAddedDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DashboardRecentlyAdded' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'recentlyAddedMedia' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DashboardBookCard' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DashboardBookCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Media' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'audio' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readProgress' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'elapsedSeconds' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DashboardRecentlyAddedQuery, DashboardRecentlyAddedQueryVariables>
export const DashboardJobsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DashboardJobs' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'jobs' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'msElapsed' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DashboardJobsQuery, DashboardJobsQueryVariables>
export const DashboardRecentAnnotationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DashboardRecentAnnotations' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pageSize' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'page' },
											value: { kind: 'IntValue', value: '1' },
										},
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'pageSize' },
											value: { kind: 'Variable', name: { kind: 'Name', value: 'pageSize' } },
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'total' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'items' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'book' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'mediaId' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	DashboardRecentAnnotationsQuery,
	DashboardRecentAnnotationsQueryVariables
>
export const AccountProfileDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'AccountProfile' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'me' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isServerOwner' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'oidcEmail' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastLogin' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'loginSessionsCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'maxSessionsAllowed' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'finishedReadingSessionsCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'permissions' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'avatar' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'preferences' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'appTheme' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'locale' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<AccountProfileQuery, AccountProfileQueryVariables>
export const AccountSignOutEverywhereDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'AccountSignOutEverywhere' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deleteUserSessions' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	AccountSignOutEverywhereMutation,
	AccountSignOutEverywhereMutationVariables
>
export const DashboardLiveEventsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'subscription',
			name: { kind: 'Name', value: 'DashboardLiveEvents' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readEvents' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: '__typename' } },
								{
									kind: 'InlineFragment',
									typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'JobStarted' } },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'id' } }],
									},
								},
								{
									kind: 'InlineFragment',
									typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'JobUpdate' } },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'subtitle' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'completedTasks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'remainingTasks' } },
										],
									},
								},
								{
									kind: 'InlineFragment',
									typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'JobOutput' } },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'id' } }],
									},
								},
								{
									kind: 'InlineFragment',
									typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'DeviceSeen' } },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
										],
									},
								},
								{
									kind: 'InlineFragment',
									typeCondition: {
										kind: 'NamedType',
										name: { kind: 'Name', value: 'ProviderSourceHealthChanged' },
									},
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{
												kind: 'Field',
												alias: { kind: 'Name', value: 'healthStatus' },
												name: { kind: 'Name', value: 'status' },
											},
										],
									},
								},
								{
									kind: 'InlineFragment',
									typeCondition: {
										kind: 'NamedType',
										name: { kind: 'Name', value: 'JobQueueStatus' },
									},
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'count' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'countByType' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	DashboardLiveEventsSubscription,
	DashboardLiveEventsSubscriptionVariables
>
export const NotificationSettingsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'NotificationSettings' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'notificationChannels' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'settings' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'valueType' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'required' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'secret' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'defaultValue' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'helpUrl' } },
										],
									},
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'notificationRules' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'eventKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'channelId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'notificationChannelSettings' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'channelId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'values' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<NotificationSettingsQuery, NotificationSettingsQueryVariables>
export const SetNotificationRuleDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetNotificationRule' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'eventKind' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'NotificationKind' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'channelId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setNotificationRule' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'eventKind' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'eventKind' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'channelId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'channelId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'enabled' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'eventKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'channelId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SetNotificationRuleMutation, SetNotificationRuleMutationVariables>
export const SetNotificationChannelSettingsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetNotificationChannelSettings' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'SetNotificationChannelSettingsInput' },
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setNotificationChannelSettings' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'channelId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'values' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	SetNotificationChannelSettingsMutation,
	SetNotificationChannelSettingsMutationVariables
>
export const TestNotificationChannelDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'TestNotificationChannel' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'channelId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'testNotificationChannel' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'channelId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'channelId' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	TestNotificationChannelMutation,
	TestNotificationChannelMutationVariables
>
export const ConsoleLibrariesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleLibraries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'libraries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'orderBy' },
								value: {
									kind: 'ListValue',
									values: [
										{
											kind: 'ObjectValue',
											fields: [
												{
													kind: 'ObjectField',
													name: { kind: 'Name', value: 'field' },
													value: { kind: 'EnumValue', value: 'NAME' },
												},
												{
													kind: 'ObjectField',
													name: { kind: 'Name', value: 'direction' },
													value: { kind: 'EnumValue', value: 'ASC' },
												},
											],
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'ConsoleLibraryCard' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'pageInfo' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsolePageInfo' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleLibraryCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Library' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastScannedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'config' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryPattern' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'watch' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'stats' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'inProgressBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalBytes' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibrariesQuery, ConsoleLibrariesQueryVariables>
export const ConsoleLibraryOptionsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleLibraryOptions' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'libraries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'offset' },
											value: {
												kind: 'ObjectValue',
												fields: [
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'page' },
														value: { kind: 'IntValue', value: '1' },
													},
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'pageSize' },
														value: { kind: 'IntValue', value: '100' },
													},
												],
											},
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibraryOptionsQuery, ConsoleLibraryOptionsQueryVariables>
export const ConsoleLibraryDetailDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleLibraryDetail' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'libraryById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsoleLibraryCard' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'tags' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleLibraryCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Library' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastScannedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'config' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryPattern' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'watch' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'stats' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'inProgressBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalBytes' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibraryDetailQuery, ConsoleLibraryDetailQueryVariables>
export const ConsoleLibrarySeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleLibrarySeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'SeriesFilterInput' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'orderBy' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'SeriesOrderBy' } },
							},
						},
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'orderBy' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'orderBy' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'ConsoleSeriesCard' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'pageInfo' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsolePageInfo' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleSeriesCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Series' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'readCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'unreadCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isComplete' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibrarySeriesQuery, ConsoleLibrarySeriesQueryVariables>
export const ConsoleBooksDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleBooks' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaFilterInput' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'orderBy' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaOrderBy' } },
							},
						},
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'media' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'orderBy' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'orderBy' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsoleBookRow' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'pageInfo' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsolePageInfo' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleBookRow' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Media' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'size' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readProgress' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readHistory' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'readthroughNumber' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'dnf' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleBooksQuery, ConsoleBooksQueryVariables>
export const ConsoleSeriesDetailDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleSeriesDetail' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'seriesById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsoleSeriesCard' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedDescription' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryId' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'library' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'config' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'libraryType' } },
													],
												},
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'stats' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'completedBooks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'inProgressBooks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'totalReadingTimeSeconds' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'metadata' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'ageRating' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'booktype' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'characters' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'collects' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'series' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'comicid' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'issueid' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'issues' } },
													],
												},
											},
											{ kind: 'Field', name: { kind: 'Name', value: 'comicImage' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'comicid' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'descriptionFormatted' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'genres' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'imprint' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'links' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'metaType' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'publicationRun' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'summary' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'totalIssues' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'volume' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'writers' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'year' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleSeriesCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Series' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'readCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'unreadCount' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'isComplete' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'thumbnail' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [{ kind: 'Field', name: { kind: 'Name', value: 'url' } }],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'metadata' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'publisher' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleSeriesDetailQuery, ConsoleSeriesDetailQueryVariables>
export const ConsoleSeriesPickerDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleSeriesPicker' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'SeriesFilterInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'series' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'orderBy' },
								value: {
									kind: 'ListValue',
									values: [
										{
											kind: 'ObjectValue',
											fields: [
												{
													kind: 'ObjectField',
													name: { kind: 'Name', value: 'series' },
													value: {
														kind: 'ObjectValue',
														fields: [
															{
																kind: 'ObjectField',
																name: { kind: 'Name', value: 'field' },
																value: { kind: 'EnumValue', value: 'NAME' },
															},
															{
																kind: 'ObjectField',
																name: { kind: 'Name', value: 'direction' },
																value: { kind: 'EnumValue', value: 'ASC' },
															},
														],
													},
												},
											],
										},
									],
								},
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'offset' },
											value: {
												kind: 'ObjectValue',
												fields: [
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'page' },
														value: { kind: 'IntValue', value: '1' },
													},
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'pageSize' },
														value: { kind: 'IntValue', value: '200' },
													},
												],
											},
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleSeriesPickerQuery, ConsoleSeriesPickerQueryVariables>
export const ConsoleAuthorsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleAuthors' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'search' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'libraryId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Pagination' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'authors' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'search' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'search' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'libraryId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'libraryId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pagination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'books' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [{ kind: 'Field', name: { kind: 'Name', value: 'id' } }],
												},
											},
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'series' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [{ kind: 'Field', name: { kind: 'Name', value: 'title' } }],
												},
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'pageInfo' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsolePageInfo' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAuthorsQuery, ConsoleAuthorsQueryVariables>
export const ConsoleLibraryPublishersDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleLibraryPublishers' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'libraryById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'publishers' },
									arguments: [
										{
											kind: 'Argument',
											name: { kind: 'Name', value: 'sort' },
											value: { kind: 'EnumValue', value: 'ASC' },
										},
									],
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleLibraryPublishersQuery, ConsoleLibraryPublishersQueryVariables>
export const ConsoleTagsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleTags' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'tags' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleTagsQuery, ConsoleTagsQueryVariables>
export const ConsoleEntityBookCountDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleEntityBookCount' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaFilterInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'media' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'filter' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'offset' },
											value: {
												kind: 'ObjectValue',
												fields: [
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'page' },
														value: { kind: 'IntValue', value: '1' },
													},
													{
														kind: 'ObjectField',
														name: { kind: 'Name', value: 'pageSize' },
														value: { kind: 'IntValue', value: '1' },
													},
												],
											},
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'pageInfo' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsolePageInfo' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsolePageInfo' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'PaginationInfo' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'InlineFragment',
						typeCondition: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'OffsetPaginationInfo' },
						},
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalItems' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalPages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'currentPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pageSize' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleEntityBookCountQuery, ConsoleEntityBookCountQueryVariables>
export const ConsoleCreateLibraryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleCreateLibrary' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'CreateOrUpdateLibraryInput' },
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createLibrary' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'ConsoleLibraryCard' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ConsoleLibraryCard' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Library' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'path' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastScannedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'config' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryType' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryPattern' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'watch' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'stats' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'completedBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'inProgressBooks' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalBytes' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleCreateLibraryMutation, ConsoleCreateLibraryMutationVariables>
export const ConsoleScanLibraryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleScanLibrary' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'scanLibrary' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleScanLibraryMutation, ConsoleScanLibraryMutationVariables>
export const ConsoleAnalyzeLibraryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleAnalyzeLibrary' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'analyzeLibrary' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleAnalyzeLibraryMutation, ConsoleAnalyzeLibraryMutationVariables>
export const ConsoleFinishMediaDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleFinishMedia' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'finishMediaProgress' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleFinishMediaMutation, ConsoleFinishMediaMutationVariables>
export const ConsoleResetMediaProgressDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleResetMediaProgress' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'clearMediaProgress' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deleteMediaReadingHistory' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleResetMediaProgressMutation,
	ConsoleResetMediaProgressMutationVariables
>
export const ConsoleFinishSeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleFinishSeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'finishSeriesProgress' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleFinishSeriesMutation, ConsoleFinishSeriesMutationVariables>
export const ConsoleClearSeriesHistoryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleClearSeriesHistory' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'clearSeriesReadingHistory' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleClearSeriesHistoryMutation,
	ConsoleClearSeriesHistoryMutationVariables
>
export const ConsoleRenameSeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleRenameSeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'SeriesMetadataInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateSeriesMetadata' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'metadata' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'title' } }],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleRenameSeriesMutation, ConsoleRenameSeriesMutationVariables>
export const ConsoleMoveMediaToSeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleMoveMediaToSeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
							},
						},
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'seriesId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'moveMediaToSeries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIds' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'seriesId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'seriesId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ConsoleMoveMediaToSeriesMutation,
	ConsoleMoveMediaToSeriesMutationVariables
>
export const ConsoleMergeSeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleMergeSeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'keep' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'drop' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mergeSeries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'keep' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'keep' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'drop' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'drop' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'droppedSeriesId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'moved' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'missingFiles' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'droppedDirectory' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'kept' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleMergeSeriesMutation, ConsoleMergeSeriesMutationVariables>
export const ConsoleSplitSeriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleSplitSeries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'ListType',
							type: {
								kind: 'NonNullType',
								type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
							},
						},
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'splitSeries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIds' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIds' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'name' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'mediaCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'libraryId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleSplitSeriesMutation, ConsoleSplitSeriesMutationVariables>
export const ConsoleKindleTargetsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleKindleTargets' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'devices' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kindleEmail' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleKindleTargetsQuery, ConsoleKindleTargetsQueryVariables>
export const ConsoleSendToKindleDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ConsoleSendToKindle' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'sendToKindle' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'deviceName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'recipient' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'converted' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleSendToKindleMutation, ConsoleSendToKindleMutationVariables>
export const ConsoleKindleDeliveriesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ConsoleKindleDeliveries' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'kindleDeliveries' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sentAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ConsoleKindleDeliveriesQuery, ConsoleKindleDeliveriesQueryVariables>
export const DevicesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'Devices' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'devices' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DevicesQuery, DevicesQueryVariables>
export const CreateDeviceDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CreateDevice' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'kind' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'DeviceKind' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createDevice' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'kind' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'kind' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'name' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'device' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'credential' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DeviceCredentialFields' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'endpoints' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DeviceEndpointFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceCredentialFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'IssuedDeviceCredential' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'credentialRef' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secret' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceEndpointFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'DeviceEndpoint' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CreateDeviceMutation, CreateDeviceMutationVariables>
export const RenameDeviceDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RenameDevice' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'renameDevice' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'name' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'name' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RenameDeviceMutation, RenameDeviceMutationVariables>
export const RotateDeviceCredentialDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RotateDeviceCredential' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'rotateDeviceCredential' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'device' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'credential' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DeviceCredentialFields' },
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'endpoints' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'DeviceEndpointFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceCredentialFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'IssuedDeviceCredential' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'credentialRef' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secret' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceEndpointFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'DeviceEndpoint' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	RotateDeviceCredentialMutation,
	RotateDeviceCredentialMutationVariables
>
export const RevokeDeviceDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RevokeDevice' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'revokeDevice' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RevokeDeviceMutation, RevokeDeviceMutationVariables>
export const SetDeviceTransformProfileDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetDeviceTransformProfile' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'profile' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'JSON' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setDeviceTransformProfile' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'profile' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'profile' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	SetDeviceTransformProfileMutation,
	SetDeviceTransformProfileMutationVariables
>
export const SetDeviceLibraryScopeDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetDeviceLibraryScope' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'libraryIds' } },
					type: {
						kind: 'ListType',
						type: {
							kind: 'NonNullType',
							type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setDeviceLibraryScope' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'libraryIds' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'libraryIds' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SetDeviceLibraryScopeMutation, SetDeviceLibraryScopeMutationVariables>
export const SetDeviceKindleEmailDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetDeviceKindleEmail' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'email' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setDeviceKindleEmail' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'email' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'email' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'DeviceFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'DeviceFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Device' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'transformProfile' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'libraryScope' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSyncSummary' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'telemetry' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryPercent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'charging' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batterySource' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'batteryObservedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncProtocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'syncedAt' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'counters' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'highlights' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'notes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookmarks' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'items' } },
										],
									},
								},
							],
						},
					},
					{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'credential' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'secretHint' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SetDeviceKindleEmailMutation, SetDeviceKindleEmailMutationVariables>
export const PendingDevicePairingsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'PendingDevicePairings' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'pendingDevicePairings' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteIp' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'failedAttempts' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'credentialIssued' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<PendingDevicePairingsQuery, PendingDevicePairingsQueryVariables>
export const ApproveDevicePairingDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ApproveDevicePairing' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pairingId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'code' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'approveDevicePairing' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pairingId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pairingId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'code' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'code' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ApproveDevicePairingMutation, ApproveDevicePairingMutationVariables>
export const DenyDevicePairingDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'DenyDevicePairing' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'pairingId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'denyDevicePairing' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pairingId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'pairingId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DenyDevicePairingMutation, DenyDevicePairingMutationVariables>
export const DeviceSeenDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'subscription',
			name: { kind: 'Name', value: 'DeviceSeen' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deviceSeen' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DeviceSeenSubscription, DeviceSeenSubscriptionVariables>
export const ReadingStatsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReadingStats' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'span' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadingStatsSpan' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readingStats' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'span' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'span' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'deviceId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'deviceId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'from' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'to' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'minutes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'booksFinished' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'streakDays' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'days' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'date' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'minutes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'devices' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'deviceId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sessions' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'minutes' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReadingStatsQuery, ReadingStatsQueryVariables>
export const MyLoginActivityDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'MyLoginActivity' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'userId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'loginActivityById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'userId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ipAddress' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userAgent' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authenticationSuccessful' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'timestamp' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<MyLoginActivityQuery, MyLoginActivityQueryVariables>
export const RuntimeComponentsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'RuntimeComponents' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'runtimeComponents' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'category' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'compiled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'desiredEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'effectiveEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'transitionMode' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'transitionReason' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'dependencies' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'health' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'restartRequired' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'usageStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'usageEvidence' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'activityCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastActivityAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastTransitionAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'ownedGauges' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'value' } },
										],
									},
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'runtimeMemory' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProcessRssBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'rssAvailable' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'rssUnavailableReason' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'anonymousPssBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'fileBackedPssBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'privateDirtyBytes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'memoryBreakdownAvailable' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'memoryBreakdownUnavailableReason' },
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RuntimeComponentsQuery, RuntimeComponentsQueryVariables>
export const SetRuntimeComponentEnabledDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetRuntimeComponentEnabled' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'key' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setRuntimeComponentEnabled' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'key' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'key' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'enabled' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'enabled' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'key' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'label' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'category' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'compiled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'desiredEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'effectiveEnabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'transitionMode' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'transitionReason' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'dependencies' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'health' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'restartRequired' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'usageStatus' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'usageEvidence' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'activityCount' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastActivityAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastTransitionAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'lastError' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'ownedGauges' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'value' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	SetRuntimeComponentEnabledMutation,
	SetRuntimeComponentEnabledMutationVariables
>
export const DeviceCapabilitiesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'DeviceCapabilities' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'deviceCapabilities' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'protocol' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'componentKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'compiled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'available' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'reason' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DeviceCapabilitiesQuery, DeviceCapabilitiesQueryVariables>
export const ReaderBookDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderBook' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mediaById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'pages' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'seriesId' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'series' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'readProgress' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'percentageCompleted' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'elapsedSeconds' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'locator' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{
															kind: 'FragmentSpread',
															name: { kind: 'Name', value: 'ReaderLocatorFields' },
														},
													],
												},
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'audio' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'codec' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'chapterSource' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'tracks' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'mime' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'startOffsetMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'byteSize' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'url' } },
													],
												},
											},
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'chapters' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'startMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'endMs' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderBookQuery, ReaderBookQueryVariables>
export const ReaderVisiblePagesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderVisiblePages' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mediaVisiblePages' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderVisiblePagesQuery, ReaderVisiblePagesQueryVariables>
export const ReaderAnnotationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderAnnotations' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'filter' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'mediaId' },
											value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
										},
									],
								},
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'pagination' },
								value: {
									kind: 'ObjectValue',
									fields: [
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'page' },
											value: { kind: 'IntValue', value: '1' },
										},
										{
											kind: 'ObjectField',
											name: { kind: 'Name', value: 'pageSize' },
											value: { kind: 'IntValue', value: '1000' },
										},
									],
								},
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'items' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'source' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'sourceDeviceName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'editable' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'fragment' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'page' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'note' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
										],
									},
								},
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'annotationsByMediaId' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'annotationText' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'locator' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'ReaderLocatorFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderAnnotationsQuery, ReaderAnnotationsQueryVariables>
export const ReaderUpdateProgressDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ReaderUpdateProgress' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'MediaProgressInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateMediaProgress' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endPage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'endPercentage' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'endLocator' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'FragmentSpread',
												name: { kind: 'Name', value: 'ReaderLocatorFields' },
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderUpdateProgressMutation, ReaderUpdateProgressMutationVariables>
export const ReaderEditionsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderEditions' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mediaById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'audio' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } }],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'editions' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'audio' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'durationMs' } },
													],
												},
											},
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'pairedPosition' },
												arguments: [
													{
														kind: 'Argument',
														name: { kind: 'Name', value: 'fromMediaId' },
														value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
													},
												],
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'sourceMediaId' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'positionMs' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'approximate' } },
														{
															kind: 'Field',
															name: { kind: 'Name', value: 'locator' },
															selectionSet: {
																kind: 'SelectionSet',
																selections: [
																	{
																		kind: 'FragmentSpread',
																		name: { kind: 'Name', value: 'ReaderLocatorFields' },
																	},
																],
															},
														},
													],
												},
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'editionSuggestions' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'evidence' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'media' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'resolvedName' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'extension' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'ReaderLocatorFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'ReadiumLocator' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'chapterTitle' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'href' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'type' } },
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'locations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'fragments' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'position' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'totalProgression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'cssSelector' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'partialCfi' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'text' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'before' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'highlight' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'after' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderEditionsQuery, ReaderEditionsQueryVariables>
export const ReaderConfirmEditionPairDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ReaderConfirmEditionPair' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdA' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdB' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'confirmEditionPair' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIdA' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdA' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIdB' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdB' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'changed' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'chapterMapEntries' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ReaderConfirmEditionPairMutation,
	ReaderConfirmEditionPairMutationVariables
>
export const ReaderRejectEditionPairDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ReaderRejectEditionPair' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdA' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdB' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'rejectEditionPair' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIdA' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdA' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mediaIdB' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mediaIdB' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'workId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'changed' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ReaderRejectEditionPairMutation,
	ReaderRejectEditionPairMutationVariables
>
export const ReaderChapterMapMediaDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderChapterMapMedia' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'mediaById' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'ebook' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'spine' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'idref' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'linear' } },
													],
												},
											},
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'audio' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'chapters' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'index' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderChapterMapMediaQuery, ReaderChapterMapMediaQueryVariables>
export const ReaderChapterMapDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'ReaderChapterMap' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'chapterMap' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'ebookMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'audioMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<ReaderChapterMapQuery, ReaderChapterMapQueryVariables>
export const ReaderSetChapterMapEntryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ReaderSetChapterMapEntry' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'ebookSpineIndex' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'audioChapterIndex' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'confidence' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Float' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setChapterMapEntry' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'ebookMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'audioMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'ebookSpineIndex' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'ebookSpineIndex' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'audioChapterIndex' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'audioChapterIndex' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'confidence' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'confidence' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioMediaId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'ebookSpineIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'audioChapterIndex' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'confidence' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ReaderSetChapterMapEntryMutation,
	ReaderSetChapterMapEntryMutationVariables
>
export const ReaderClearChapterMapEntryDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ReaderClearChapterMapEntry' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'ebookSpineIndex' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'clearChapterMapEntry' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'ebookMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'ebookMediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'audioMediaId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'audioMediaId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'ebookSpineIndex' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'ebookSpineIndex' } },
							},
						],
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	ReaderClearChapterMapEntryMutation,
	ReaderClearChapterMapEntryMutationVariables
>
export const BookRequestsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookRequests' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'status' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestStatus' } },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'mineOnly' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: false },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					defaultValue: { kind: 'IntValue', value: '50' },
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					defaultValue: { kind: 'IntValue', value: '0' },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookRequests' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'status' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'status' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'mineOnly' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'mineOnly' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'offset' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'offset' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestsQuery, BookRequestsQueryVariables>
export const RequestDestinationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'RequestDestinations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'devices' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readingLists' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RequestDestinationsQuery, RequestDestinationsQueryVariables>
export const BookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestQuery, BookRequestQueryVariables>
export const BookRequestReleasesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookRequestReleases' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookRequestReleases' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookRequestReleaseFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestReleaseFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestRelease' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'searchId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'format' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'language' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'edition' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'quality' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sizeBytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'seeders' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewName' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewMime' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'previewBytes' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'score' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoreComponents' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rank' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'selected' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestReleasesQuery, BookRequestReleasesQueryVariables>
export const BookRequestGatewayDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'BookRequestGateway' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookRequestGateway' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookRequestGatewayFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGatewayFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'BookRequestGatewaySettings' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'endpoint' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requireApproval' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'handoffRoot' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hasToken' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'tokenRedacted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<BookRequestGatewayQuery, BookRequestGatewayQueryVariables>
export const CreateBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CreateBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'CreateBookRequestInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CreateBookRequestMutation, CreateBookRequestMutationVariables>
export const ApproveBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'ApproveBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'reason' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'approveBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'reason' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'reason' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<ApproveBookRequestMutation, ApproveBookRequestMutationVariables>
export const RejectBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RejectBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'reason' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'rejectBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'reason' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'reason' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<RejectBookRequestMutation, RejectBookRequestMutationVariables>
export const SearchBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SearchBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'searchBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<SearchBookRequestMutation, SearchBookRequestMutationVariables>
export const SelectBookRequestReleaseDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SelectBookRequestRelease' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'releaseId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'selectBookRequestRelease' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'releaseId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'releaseId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	SelectBookRequestReleaseMutation,
	SelectBookRequestReleaseMutationVariables
>
export const GrabBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'GrabBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'grabBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestGrabFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGrabFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestGrab' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'releaseId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'opaqueId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastPolledAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextPollAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<GrabBookRequestMutation, GrabBookRequestMutationVariables>
export const PollBookRequestGrabDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'PollBookRequestGrab' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'grabId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'pollBookRequestGrab' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'grabId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'grabId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestGrabFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGrabFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestGrab' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'releaseId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'opaqueId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'attempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxAttempts' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastPolledAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'nextPollAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<PollBookRequestGrabMutation, PollBookRequestGrabMutationVariables>
export const RetryBookRequestDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RetryBookRequest' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'retryBookRequest' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'requestId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'requestId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'BookRequestFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequest' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requesterId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalMediaId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'internalWorkId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'destinationDeviceId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvalPolicy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'retries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'rejectedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureCode' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'failureMessage' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'approvedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'completedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<RetryBookRequestMutation, RetryBookRequestMutationVariables>
export const UpdateBookRequestGatewayDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'UpdateBookRequestGateway' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookRequestGatewayInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'updateBookRequestGateway' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'FragmentSpread',
									name: { kind: 'Name', value: 'BookRequestGatewayFields' },
								},
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'BookRequestGatewayFields' },
			typeCondition: {
				kind: 'NamedType',
				name: { kind: 'Name', value: 'BookRequestGatewaySettings' },
			},
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'endpoint' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'enabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'requireApproval' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'automationEnabled' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'scoringFloor' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'verificationThreshold' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'maxRetries' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'handoffRoot' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'hasToken' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'tokenRedacted' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedBy' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<
	UpdateBookRequestGatewayMutation,
	UpdateBookRequestGatewayMutationVariables
>
export const IncomingSocialRecommendationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'IncomingSocialRecommendations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialRecommendations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'direction' },
								value: { kind: 'EnumValue', value: 'INCOMING' },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'acceptedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'handoffState' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	IncomingSocialRecommendationsQuery,
	IncomingSocialRecommendationsQueryVariables
>
export const OutgoingSocialRecommendationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'OutgoingSocialRecommendations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialRecommendations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'direction' },
								value: { kind: 'EnumValue', value: 'OUTGOING' },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'acceptedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'handoffState' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	OutgoingSocialRecommendationsQuery,
	OutgoingSocialRecommendationsQueryVariables
>
export const AdaptiveRecommendationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'AdaptiveRecommendations' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Int' } },
					defaultValue: { kind: 'IntValue', value: '20' },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'adaptiveRecommendations' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'limit' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'reasonCode' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'score' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<AdaptiveRecommendationsQuery, AdaptiveRecommendationsQueryVariables>
export const SocialShareGrantsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialShareGrants' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'incoming' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: true },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialShareGrants' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'incoming' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'incoming' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SocialShareGrantsQuery, SocialShareGrantsQueryVariables>
export const SocialOverlaysDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialOverlays' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'targetKey' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialOverlays' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'targetKeyFilter' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'targetKey' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'body' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'capturedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'hidden' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SocialOverlaysQuery, SocialOverlaysQueryVariables>
export const SocialUserSearchDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialUserSearch' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'query' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialUserSearch' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'query' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'query' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SocialUserSearchQuery, SocialUserSearchQueryVariables>
export const SocialPreferencesDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialPreferences' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'socialPreferences' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'recommendationsOptOut' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sharingOptOut' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SocialPreferencesQuery, SocialPreferencesQueryVariables>
export const SendRecommendationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SendRecommendation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'SendRecommendationInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'sendRecommendation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'handoffState' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SendRecommendationMutation, SendRecommendationMutationVariables>
export const RespondToRecommendationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RespondToRecommendation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'response' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'RecommendationResponseInput' },
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'respondToRecommendation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'response' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'response' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'acceptedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'handoffState' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	RespondToRecommendationMutation,
	RespondToRecommendationMutationVariables
>
export const RevokeRecommendationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RevokeRecommendation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'revokeRecommendation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RevokeRecommendationMutation, RevokeRecommendationMutationVariables>
export const DismissRecommendationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'DismissRecommendation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'dismissRecommendation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<DismissRecommendationMutation, DismissRecommendationMutationVariables>
export const RequestRecommendationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RequestRecommendation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'destination' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'RequestDestinationInput' } },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'requestRecommendation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'destination' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'destination' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'direction' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'coverUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'acceptedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'handoffState' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'requestId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'destinationShelfId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RequestRecommendationMutation, RequestRecommendationMutationVariables>
export const SetRecommendationOptOutDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetRecommendationOptOut' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'optOut' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setRecommendationOptOut' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'optOut' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'optOut' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'recommendationsOptOut' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sharingOptOut' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'updatedAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	SetRecommendationOptOutMutation,
	SetRecommendationOptOutMutationVariables
>
export const CreateShareGrantDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CreateShareGrant' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'CreateShareGrantInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createShareGrant' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'sourceProvider' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'remoteId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'externalKey' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<CreateShareGrantMutation, CreateShareGrantMutationVariables>
export const RespondToShareGrantDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RespondToShareGrant' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'accept' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'respondToShareGrant' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'accept' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'accept' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'title' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'authors' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'scopes' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'expiresAt' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RespondToShareGrantMutation, RespondToShareGrantMutationVariables>
export const RevokeShareGrantDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RevokeShareGrant' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'revokeShareGrant' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'state' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RevokeShareGrantMutation, RevokeShareGrantMutationVariables>
export const SetOverlayVisibilityDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'SetOverlayVisibility' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'overlayId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'hidden' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'setOverlayVisibility' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'overlayId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'overlayId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'hidden' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'hidden' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'targetKey' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'excerpt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'body' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'progression' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'percentage' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'color' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'capturedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'hidden' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SetOverlayVisibilityMutation, SetOverlayVisibilityMutationVariables>
export const SocialRequestDestinationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialRequestDestinations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'devices' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'revokedAt' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readingLists' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'nodes' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	SocialRequestDestinationsQuery,
	SocialRequestDestinationsQueryVariables
>
export const SocialBookClubsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'SocialBookClubs' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'all' } },
					type: { kind: 'NamedType', name: { kind: 'Name', value: 'Boolean' } },
					defaultValue: { kind: 'BooleanValue', value: false },
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'bookClubs' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'all' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'all' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'slug' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'membersCount' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'membership' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'displayName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'avatarUrl' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'hideProgress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'joinedAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'isCreator' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'members' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'displayName' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'avatarUrl' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'hideProgress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'joinedAt' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'isCreator' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'invitations' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'bookClubId' } },
											{
												kind: 'Field',
												name: { kind: 'Name', value: 'user' },
												selectionSet: {
													kind: 'SelectionSet',
													selections: [
														{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
														{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
													],
												},
											},
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<SocialBookClubsQuery, SocialBookClubsQueryVariables>
export const MyBookClubInvitationsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'MyBookClubInvitations' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'myBookClubInvitations' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookClubId' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'user' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
										],
									},
								},
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'bookClub' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'slug' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<MyBookClubInvitationsQuery, MyBookClubInvitationsQueryVariables>
export const CreateBookClubInvitationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CreateBookClubInvitation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'BookClubInvitationInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createBookClubInvitation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookClubId' } },
								{
									kind: 'Field',
									name: { kind: 'Name', value: 'user' },
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	CreateBookClubInvitationMutation,
	CreateBookClubInvitationMutationVariables
>
export const RespondToBookClubInvitationDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RespondToBookClubInvitation' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: {
							kind: 'NamedType',
							name: { kind: 'Name', value: 'BookClubInvitationResponseInput' },
						},
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'respondToBookClubInvitation' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookClubId' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<
	RespondToBookClubInvitationMutation,
	RespondToBookClubInvitationMutationVariables
>
export const RemoveBookClubMemberDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'RemoveBookClubMember' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'bookClubId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'memberId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'removeBookClubMember' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'bookClubId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'bookClubId' } },
							},
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'memberId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'memberId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'username' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'displayName' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'avatarUrl' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'hideProgress' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'joinedAt' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isCreator' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<RemoveBookClubMemberMutation, RemoveBookClubMemberMutationVariables>
export const LeaveBookClubDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'LeaveBookClub' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'bookClubId' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'ID' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'leaveBookClub' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'bookClubId' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'bookClubId' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'userId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'bookClubId' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'role' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<LeaveBookClubMutation, LeaveBookClubMutationVariables>
export const CreateBookClubDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CreateBookClub' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'CreateBookClubInput' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'createBookClub' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'input' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'input' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'slug' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'description' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'isPrivate' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'emoji' } },
								{ kind: 'Field', name: { kind: 'Name', value: 'membersCount' } },
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<CreateBookClubMutation, CreateBookClubMutationVariables>
export const WorkersDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'query',
			name: { kind: 'Name', value: 'Workers' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'workers' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'WorkerFields' } },
							],
						},
					},
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'workerJobs' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'limit' },
								value: { kind: 'IntValue', value: '50' },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'WorkerJobFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'WorkerFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'Worker' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'name' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'version' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kinds' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'connectedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'lastSeenAt' } },
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'WorkerJobFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'WorkerJob' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workerId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'priority' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<WorkersQuery, WorkersQueryVariables>
export const CancelWorkerJobDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'mutation',
			name: { kind: 'Name', value: 'CancelWorkerJob' },
			variableDefinitions: [
				{
					kind: 'VariableDefinition',
					variable: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
					type: {
						kind: 'NonNullType',
						type: { kind: 'NamedType', name: { kind: 'Name', value: 'String' } },
					},
				},
			],
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'cancelWorkerJob' },
						arguments: [
							{
								kind: 'Argument',
								name: { kind: 'Name', value: 'id' },
								value: { kind: 'Variable', name: { kind: 'Name', value: 'id' } },
							},
						],
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'FragmentSpread', name: { kind: 'Name', value: 'WorkerJobFields' } },
							],
						},
					},
				],
			},
		},
		{
			kind: 'FragmentDefinition',
			name: { kind: 'Name', value: 'WorkerJobFields' },
			typeCondition: { kind: 'NamedType', name: { kind: 'Name', value: 'WorkerJob' } },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'workerId' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'priority' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'createdAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'startedAt' } },
					{ kind: 'Field', name: { kind: 'Name', value: 'finishedAt' } },
				],
			},
		},
	],
} as unknown as DocumentNode<CancelWorkerJobMutation, CancelWorkerJobMutationVariables>
export const WorkerJobEventsDocument = {
	kind: 'Document',
	definitions: [
		{
			kind: 'OperationDefinition',
			operation: 'subscription',
			name: { kind: 'Name', value: 'WorkerJobEvents' },
			selectionSet: {
				kind: 'SelectionSet',
				selections: [
					{
						kind: 'Field',
						name: { kind: 'Name', value: 'readEvents' },
						selectionSet: {
							kind: 'SelectionSet',
							selections: [
								{ kind: 'Field', name: { kind: 'Name', value: '__typename' } },
								{
									kind: 'InlineFragment',
									typeCondition: {
										kind: 'NamedType',
										name: { kind: 'Name', value: 'WorkerJobChanged' },
									},
									selectionSet: {
										kind: 'SelectionSet',
										selections: [
											{ kind: 'Field', name: { kind: 'Name', value: 'id' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'kind' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'status' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'workerId' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'progress' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'message' } },
											{ kind: 'Field', name: { kind: 'Name', value: 'error' } },
										],
									},
								},
							],
						},
					},
				],
			},
		},
	],
} as unknown as DocumentNode<WorkerJobEventsSubscription, WorkerJobEventsSubscriptionVariables>
