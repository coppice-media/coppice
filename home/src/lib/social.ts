import { parse, type DocumentNode } from 'graphql'
import type { TypedDocumentNode } from '@graphql-typed-document-node/core'
import { request } from '@stump/ui/graphql/client'
import socialSource from '$lib/graphql/social.graphql?raw'

/**
 * The social document source is kept in one `.graphql` file so codegen can
 * publish typed documents when the server schema is available. Until then,
 * Home selects those named operations from the source and keeps only the
 * narrow response types it needs to render safely.
 */
export type SocialVariables = Record<string, unknown>

const parsedSocialDocument = parse(socialSource)
const socialOperations = new Map<string, DocumentNode>()
for (const definition of parsedSocialDocument.definitions) {
	if (definition.kind === 'OperationDefinition' && definition.name?.value) {
		socialOperations.set(definition.name.value, {
			kind: 'Document',
			definitions: [definition],
		})
	}
}

function operation<TResult>(name: string): TypedDocumentNode<TResult, SocialVariables> {
	const document = socialOperations.get(name)
	if (!document) throw new Error(`Social operation ${name} is not available.`)
	return document as TypedDocumentNode<TResult, SocialVariables>
}

export function socialRequest<TResult>(
	document: TypedDocumentNode<TResult, SocialVariables>,
	variables: SocialVariables = {},
): Promise<TResult> {
	return request(document, variables)
}

export type RecommendationState =
	| 'PENDING'
	| 'ACCEPTED'
	| 'DECLINED'
	| 'REVOKED'
	| 'EXPIRED'
	| 'DISMISSED'
	| string

export type RecommendationHandoffState = 'NONE' | 'REQUESTED' | 'LINKED' | 'FAILED' | string

export interface SocialRecommendation {
	id: string
	direction: 'INCOMING' | 'OUTGOING'
	targetKind: string
	sourceProvider?: string | null
	remoteId?: string | null
	externalKey?: string | null
	title: string
	authors: string
	coverUrl?: string | null
	message?: string | null
	state: RecommendationState
	handoffState?: RecommendationHandoffState | null
	requestId?: string | null
	destinationShelfId?: string | null
	createdAt: string
	expiresAt?: string | null
	acceptedAt?: string | null
}

export interface AdaptiveRecommendation {
	targetKey: string
	title: string
	authors: string
	coverUrl?: string | null
	reasonCode?: string | null
	score: number
}

export interface SocialRecipient {
	id: string
	username: string
}

export type ShareScope =
	| 'METADATA'
	| 'RATING'
	| 'REVIEW_TEXT'
	| 'COMPLETION'
	| 'PERCENTAGE'
	| 'ANNOTATIONS'
	| string

export interface SocialShareGrant {
	id: string
	targetKey: string
	title: string
	authors: string
	scopes: ShareScope[]
	state: 'PENDING' | 'ACTIVE' | 'DECLINED' | 'REVOKED' | 'EXPIRED' | string
	expiresAt?: string | null
	sourceProvider?: string | null
	remoteId?: string | null
	externalKey?: string | null
}

export type LiseurColor = 'yellow' | 'green' | 'blue' | 'pink' | 'purple' | 'orange'

export interface SocialShareOverlay {
	id: string
	targetKey: string
	kind: 'HIGHLIGHT' | 'NOTE' | 'BOOKMARK' | string
	excerpt?: string | null
	body?: string | null
	progression?: number | null
	percentage?: number | null
	color?: LiseurColor | null
	capturedAt: string
	hidden: boolean
}

export interface SocialPreferences {
	recommendationsOptOut: boolean
	sharingOptOut: boolean
	updatedAt?: string | null
}

export interface SocialRequestDestination {
	id: string
	name: string
	revokedAt?: string | null
}

export interface SocialRequestDestinations {
	devices: SocialRequestDestination[]
	readingLists?: { nodes: Array<{ id: string; name: string }> } | null
}

export interface BookClubMember {
	id: string
	userId: string
	username: string
	displayName?: string | null
	avatarUrl?: string | null
	role: string
	hideProgress: boolean
	joinedAt: string
	isCreator: boolean
}

export interface BookClubInvitation {
	id: string
	role: string
	userId: string
	bookClubId: string
	user?: { id: string; username: string } | null
}

export interface BookClub {
	id: string
	name: string
	slug: string
	description?: string | null
	isPrivate: boolean
	emoji?: string | null
	membersCount: number
	membership?: BookClubMember | null
	members: BookClubMember[]
	invitations: BookClubInvitation[]
}

export const IncomingSocialRecommendationsDocument = operation<{
	socialRecommendations: SocialRecommendation[]
}>('IncomingSocialRecommendations')

export const OutgoingSocialRecommendationsDocument = operation<{
	socialRecommendations: SocialRecommendation[]
}>('OutgoingSocialRecommendations')
export const AdaptiveRecommendationsDocument = operation<{
	adaptiveRecommendations: AdaptiveRecommendation[]
}>('AdaptiveRecommendations')

export const SocialRecipientsDocument = operation<{
	socialUserSearch: SocialRecipient[]
}>('SocialUserSearch')

export const SocialPreferencesDocument = operation<{
	socialPreferences: SocialPreferences
}>('SocialPreferences')

export const SendRecommendationDocument = operation<{
	sendRecommendation: SocialRecommendation
}>('SendRecommendation')

export const RespondToRecommendationDocument = operation<{
	respondToRecommendation: SocialRecommendation
}>('RespondToRecommendation')

export const RevokeRecommendationDocument = operation<{
	revokeRecommendation: SocialRecommendation
}>('RevokeRecommendation')

export const DismissRecommendationDocument = operation<{
	dismissRecommendation: SocialRecommendation
}>('DismissRecommendation')

export const RequestRecommendationDocument = operation<{
	requestRecommendation: SocialRecommendation
}>('RequestRecommendation')

export const SetRecommendationOptOutDocument = operation<{
	setRecommendationOptOut: SocialPreferences
}>('SetRecommendationOptOut')

export const SocialRequestDestinationsDocument = operation<SocialRequestDestinations>(
	'SocialRequestDestinations',
)

export const SocialShareGrantsDocument = operation<{
	socialShareGrants: SocialShareGrant[]
}>('SocialShareGrants')

export const CreateShareGrantDocument = operation<{
	createShareGrant: SocialShareGrant
}>('CreateShareGrant')

export const RespondToShareGrantDocument = operation<{
	respondToShareGrant: SocialShareGrant
}>('RespondToShareGrant')

export const RevokeShareGrantDocument = operation<{
	revokeShareGrant: SocialShareGrant
}>('RevokeShareGrant')

export const SocialShareOverlaysDocument = operation<{
	socialOverlays: SocialShareOverlay[]
}>('SocialOverlays')

export const SetOverlayVisibilityDocument = operation<{
	setOverlayVisibility: SocialShareOverlay
}>('SetOverlayVisibility')

export const BookClubsDocument = operation<{ bookClubs: BookClub[] }>('SocialBookClubs')

export const MyBookClubInvitationsDocument = operation<{
	myBookClubInvitations: BookClubInvitation[]
}>('MyBookClubInvitations')

export const CreateBookClubInvitationDocument = operation<{
	createBookClubInvitation: BookClubInvitation
}>('CreateBookClubInvitation')

export const RespondToBookClubInvitationDocument = operation<{
	respondToBookClubInvitation: BookClubInvitation
}>('RespondToBookClubInvitation')

export const RemoveBookClubMemberDocument = operation<{
	removeBookClubMember: BookClubMember
}>('RemoveBookClubMember')

export const LeaveBookClubDocument = operation<{
	leaveBookClub: BookClubMember
}>('LeaveBookClub')

export const CreateBookClubDocument = operation<{
	createBookClub: BookClub
}>('CreateBookClub')

export const SOCIAL_SCOPES = [
	{ key: 'METADATA', label: 'Book metadata', description: 'Title, authors, and cover' },
	{ key: 'RATING', label: 'Rating', description: 'Your rating only' },
	{ key: 'REVIEW_TEXT', label: 'Review text', description: 'Your non-private review text' },
	{ key: 'COMPLETION', label: 'Completion', description: 'A coarse finished/not-finished flag' },
	{ key: 'PERCENTAGE', label: 'Progress percentage', description: 'Rounded percentage only' },
	{
		key: 'ANNOTATIONS',
		label: 'Highlights and notes',
		description: 'Immutable shared annotation snapshots',
	},
] as const

export const LISEUR_COLORS: Record<LiseurColor, { label: string; hex: string }> = {
	yellow: { label: 'Yellow', hex: '#FFD54F' },
	green: { label: 'Green', hex: '#9CCC65' },
	blue: { label: 'Blue', hex: '#64B5F6' },
	pink: { label: 'Pink', hex: '#F06292' },
	purple: { label: 'Purple', hex: '#B39DDB' },
	orange: { label: 'Orange', hex: '#FFB74D' },
}

export function scopeNames(scopes: readonly string[]): string[] {
	return scopes
		.map((scope) => SOCIAL_SCOPES.find((candidate) => candidate.key === scope)?.label ?? scope)
		.filter((label, index, labels) => labels.indexOf(label) === index)
}

export function recommendationReason(
	recommendation: SocialRecommendation,
	adaptive: readonly AdaptiveRecommendation[] = [],
): string {
	const match = adaptive.find((candidate) => candidate.targetKey === recommendation.externalKey)
	if (match?.reasonCode === 'SIMILAR_TO_READING') return 'Similar to books you have enjoyed'
	if (match?.reasonCode === 'UNFINISHED_SERIES') return 'Continues a series you started'
	if (match?.reasonCode === 'ADAPTIVE_MATCH') return 'Matches your recent reading pattern'
	if (match?.reasonCode) return 'Selected from your reading preferences'
	return 'Shared directly with you'
}

export function formatAuthors(authors: string): string {
	try {
		const parsed = JSON.parse(authors)
		if (Array.isArray(parsed))
			return parsed.filter((author) => typeof author === 'string').join(', ')
	} catch {
		// Old snapshots may store a plain display string.
	}
	return authors
}

export function errorMessage(error: unknown, fallback: string): string {
	return error instanceof Error && error.message ? error.message : fallback
}
