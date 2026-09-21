import { parse } from 'graphql'
import type { TypedDocumentNode } from '@graphql-typed-document-node/core'
import type { SharedCandidateLike } from '../editor-fields'

export type MetadataSearchInput = {
	title?: string
	author?: string
	isbn?: string
	limit?: number
}

export type MetadataSearchCandidate = {
	provider: string
	externalId: string
	confidence: number
	metadata?: {
		provider?: string | null
		externalId?: string | null
		title?: string | null
		summary?: string | null
		pageCount?: number | null
		seriesName?: string | null
		number?: unknown
		day?: number | null
		month?: number | null
		year?: number | null
		genres?: string[] | null
		tags?: string[] | null
		isbn?: string | null
		isbn13?: string | null
		writers?: string[] | null
		artists?: string[] | null
		colorists?: string[] | null
		letterers?: string[] | null
		coverArtists?: string[] | null
		coverUrl?: string | null
		providerUrl?: string | null
		subtitle?: string | null
		narrators?: string[] | null
		publisher?: string | null
		runtimeMinutes?: number | null
	} | null
}

export type MetadataSearchResult = {
	searchBookMetadata?: {
		matchCandidates?: MetadataSearchCandidate[] | null
	} | null
}

/** The resolver shared by Home and the standalone media editor. */
export const SearchBookMetadataDocument = parse(/* GraphQL */ `
	query SearchBookMetadata($mediaId: ID!, $search: MediaMetadataSearchInput) {
		searchBookMetadata(mediaId: $mediaId, search: $search) {
			matchCandidates {
				provider
				externalId
				confidence
				metadata {
					... on ExternalMediaMetadata {
						provider
						externalId
						title
						summary
						pageCount
						seriesName
						number
						day
						month
						year
						genres
						tags
						isbn
						isbn13
						writers
						artists
						colorists
						letterers
						coverArtists
						coverUrl
						providerUrl
						subtitle
						narrators
						publisher
						runtimeMinutes
					}
				}
			}
		}
	}
`) as unknown as TypedDocumentNode<
	MetadataSearchResult,
	{ mediaId: string; search?: MetadataSearchInput | null }
>

/** Translate the book-detail resolver's external candidate shape once for both apps. */
export function metadataSearchCandidateToShared(
	candidate: MetadataSearchCandidate,
): SharedCandidateLike {
	const metadata = candidate.metadata ?? {}
	const shared: SharedCandidateLike & { source: 'external' } = {
		id: `${candidate.provider}:${candidate.externalId}`,
		provider: candidate.provider,
		providerVersion: 'External metadata',
		confidence: candidate.confidence,
		status: 'AVAILABLE',
		createdAt: null,
		source: 'external',
		fields: {
			TITLE: metadata.title,
			SUMMARY: metadata.summary,
			PAGE_COUNT: metadata.pageCount,
			SERIES: metadata.seriesName,
			SERIES_INDEX: metadata.number,
			PUBLISHED_DATE: { year: metadata.year, month: metadata.month, day: metadata.day },
			GENRES: metadata.genres,
			TAGS: metadata.tags,
			ISBN: metadata.isbn13 ?? metadata.isbn,
			AUTHORS: metadata.writers ?? metadata.artists,
			NARRATORS: metadata.narrators,
			PUBLISHER: metadata.publisher,
			COVER_URL: metadata.coverUrl,
			IDENTIFIERS: candidate.externalId,
		},
	}
	return shared
}
